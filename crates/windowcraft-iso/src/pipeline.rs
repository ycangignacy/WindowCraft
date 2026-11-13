// Made by WindowCraft Dev Team
//! ISO Studio pipeline: select, configure, export.
//!
//! The work directory persists between steps. `session.json` records the last
//! completed step, so an interrupted build resumes instead of starting over.

use crate::answer::{AnswerOptions, autounattend, ei_cfg, setup_complete};
use crate::drivers;
use crate::error::{IsoError, Result};
use crate::mount::MountedIso;
use crate::usb::{DiskIdentity, SWM_PART_BYTES, prepare_disk};
use crate::wim::{ImageInfo, WimFile};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windowcraft_config::{Catalog, Profile};
use windowcraft_engine::dism::{MountedImage, cleanup_mountpoints};
use windowcraft_engine::download::{block_on, download};
use windowcraft_engine::jobs::JobContext;
use windowcraft_engine::process::run_ok;

/// A progress step of ISO Studio. The app names it in the user's language
/// through `set_step_names`; without that, for example in tests, it is English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoStep {
    Mounting,
    CopyingMedia,
    WritingAnswerFile,
    ExportingDrivers,
    InjectingBoot(usize),
    ConvertingEsd,
    InjectingInstall(usize),
    InstallingOscdimg,
    BuildingIso,
    SplittingWim,
    CopyingToUsb,
    Partitioning,
}

impl IsoStep {
    pub fn english(self) -> String {
        match self {
            Self::Mounting => "Mounting ISO".to_owned(),
            Self::CopyingMedia => "Copying installation media".to_owned(),
            Self::WritingAnswerFile => "Writing answer file and first-boot scripts".to_owned(),
            Self::ExportingDrivers => "Exporting system drivers".to_owned(),
            Self::InjectingBoot(count) => format!("Injecting {count} storage drivers into boot.wim"),
            Self::ConvertingEsd => "Converting install.esd to install.wim".to_owned(),
            Self::InjectingInstall(count) => format!("Injecting {count} drivers into install.wim"),
            Self::InstallingOscdimg => "Installing oscdimg".to_owned(),
            Self::BuildingIso => "Building ISO".to_owned(),
            Self::SplittingWim => "Splitting install.wim".to_owned(),
            Self::CopyingToUsb => "Copying files to USB".to_owned(),
            Self::Partitioning => "Partitioning USB drive".to_owned(),
        }
    }
}

static STEP_NAMES: std::sync::OnceLock<fn(IsoStep) -> String> = std::sync::OnceLock::new();

/// Sets how steps are named in progress and in the job log. The first call wins.
pub fn set_step_names(names: fn(IsoStep) -> String) {
    let _ = STEP_NAMES.set(names);
}

fn step(context: &JobContext, fraction: f32, step: IsoStep) {
    let name = STEP_NAMES.get().map_or_else(|| step.english(), |names| names(step));
    context.progress(fraction, name);
}

const BOOT_IMAGE_INDEX: u32 = 2;
const OSCDIMG_ID: &str = "Microsoft.OSCDIMG";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Step {
    Created,
    Extracted,
    Configured,
    Exported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioSession {
    pub source_iso: PathBuf,
    pub step: Step,
    pub options: AnswerOptions,
    pub inject_drivers: bool,
    pub profile: Option<Profile>,
}

impl StudioSession {
    pub fn new(source_iso: PathBuf, options: AnswerOptions) -> Self {
        Self { source_iso, step: Step::Created, options, inject_drivers: false, profile: None }
    }

    fn path(work: &Path) -> PathBuf {
        work.join("session.json")
    }

    pub fn save(&self, work: &Path) -> Result<()> {
        fs::create_dir_all(work).map_err(|e| IsoError::io(work.display().to_string(), e))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| IsoError::Invalid(e.to_string()))?;
        windowcraft_engine::atomicfs::atomic_write(&Self::path(work), text.as_bytes())?;
        Ok(())
    }

    /// Reloads an interrupted session.
    pub fn resume(work: &Path) -> Option<Self> {
        serde_json::from_str(&fs::read_to_string(Self::path(work)).ok()?).ok()
    }
}

/// Finds the install image in an extracted media tree.
pub fn install_image(media: &Path) -> Option<PathBuf> {
    ["install.wim", "install.esd"].iter().map(|name| media.join("sources").join(name)).find(|path| path.exists())
}

/// Confirms the media holds a Windows 11 install image and lists its editions.
pub fn validate_media(media: &Path) -> Result<Vec<ImageInfo>> {
    let image = install_image(media)
        .ok_or_else(|| IsoError::Invalid("no sources\\install.wim or install.esd on this media".to_owned()))?;
    let images = WimFile::open(&image)?.images()?;
    if images.iter().any(ImageInfo::is_windows_11) {
        Ok(images)
    } else {
        Err(IsoError::Invalid("this media does not contain a Windows 11 image".to_owned()))
    }
}

/// Picks the Pro image when present, otherwise the first one.
pub fn default_edition(images: &[ImageInfo]) -> Option<&ImageInfo> {
    images.iter().find(|image| image.edition == "Professional").or_else(|| images.first())
}

/// Pins an edition, including the architecture the answer file must name.
/// Components with the wrong `processorArchitecture` are silently ignored.
pub fn select_edition(options: &mut AnswerOptions, image: &ImageInfo) {
    options.image_index = image.index;
    options.edition_id = image.edition.clone();
    options.architecture = match image.architecture.as_str() {
        "ARM64" => "arm64",
        _ => "amd64",
    }
    .to_owned();
}

/// Points the session at the chosen edition in the current install image. An
/// earlier build may have turned install.esd into a one-image install.wim,
/// which moves the edition to index 1.
fn align_edition(session: &mut StudioSession, images: &[ImageInfo]) -> Result<()> {
    let wanted = &session.options.edition_id;
    let at_index = images.iter().find(|image| image.index == session.options.image_index);
    if at_index.is_some_and(|image| wanted.is_empty() || image.edition == *wanted) {
        return Ok(());
    }
    match images.iter().find(|image| image.edition == *wanted) {
        Some(image) => {
            select_edition(&mut session.options, image);
            Ok(())
        }
        None => Err(IsoError::Invalid(format!(
            "the work media no longer holds the {wanted} edition; open the ISO again to choose another one"
        ))),
    }
}

/// Copies a directory tree in parallel, reporting bytes as they land.
pub fn copy_tree(source: &Path, destination: &Path, context: Option<&JobContext>) -> Result<()> {
    fs::create_dir_all(destination).map_err(|e| IsoError::io(destination.display().to_string(), e))?;
    let entries: Vec<_> =
        fs::read_dir(source).map_err(|e| IsoError::io(source.display().to_string(), e))?.flatten().collect();
    entries.par_iter().try_for_each(|entry| {
        if let Some(context) = context {
            context.cancel.check().map_err(IsoError::Engine)?;
        }
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| IsoError::io(entry.path().display().to_string(), e))?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, context)
        } else {
            fs::copy(entry.path(), &target).map(drop).map_err(|e| IsoError::io(target.display().to_string(), e))?;
            clear_read_only(&target)
        }
    })
}

/// Files copied from an ISO arrive read-only.
fn clear_read_only(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path).map_err(|e| IsoError::io(path.display().to_string(), e))?;
    let mut permissions = metadata.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).map_err(|e| IsoError::io(path.display().to_string(), e))
}

/// Deletes a media tree left by an earlier ISO. Read-only flags are cleared
/// first, and links are removed without entering them.
fn remove_media(media: &Path) -> Result<()> {
    fn clear(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path).map_err(|e| IsoError::io(path.display().to_string(), e))?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for entry in fs::read_dir(path).map_err(|e| IsoError::io(path.display().to_string(), e))?.flatten() {
                clear(&entry.path())?;
            }
        } else if metadata.permissions().readonly() {
            clear_read_only(path)?;
        }
        Ok(())
    }
    match fs::symlink_metadata(media) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(IsoError::io(media.display().to_string(), error)),
        Ok(_) => {}
    }
    clear(media)?;
    fs::remove_dir_all(media).map_err(|e| IsoError::io(media.display().to_string(), e))
}

/// Replaces the media tree with a full copy of `source`, so no file of an
/// earlier ISO stays behind.
fn copy_media(source: &Path, media: &Path, context: Option<&JobContext>) -> Result<()> {
    remove_media(media)?;
    copy_tree(source, media, context)
}

/// Step one: extract the ISO into the work directory.
pub fn extract(session: &mut StudioSession, work: &Path, context: &JobContext) -> Result<Vec<ImageInfo>> {
    let media = work.join("media");
    if session.step < Step::Extracted {
        step(context, 0.05, IsoStep::Mounting);
        let mounted = MountedIso::attach(&session.source_iso)?;
        step(context, 0.1, IsoStep::CopyingMedia);
        copy_media(mounted.root(), &media, Some(context))?;
        drop(mounted);
        session.step = Step::Extracted;
        session.save(work)?;
    }
    validate_media(&media)
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| IsoError::io(parent.display().to_string(), e))?;
    }
    fs::write(path, text).map_err(|e| IsoError::io(path.display().to_string(), e))
}

/// Writes every staged configuration file into the media tree.
pub fn stage_files(media: &Path, session: &StudioSession, catalog: &Catalog) -> Result<()> {
    let options = AnswerOptions { run_profile_at_logon: session.profile.is_some(), ..session.options.clone() };
    write_text(&media.join("autounattend.xml"), &autounattend(&options))?;
    write_text(&media.join(r"sources\ei.cfg"), &ei_cfg(&session.options.edition_id))?;
    match fs::remove_file(media.join(r"sources\PID.txt")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(IsoError::io("sources\\PID.txt", error));
        }
        _ => {}
    }

    let scripts = media.join(r"sources\$OEM$\$$\Setup\Scripts");
    write_text(&scripts.join("SetupComplete.cmd"), &setup_complete(catalog, &session.options))?;
    if let Some(profile) = &session.profile {
        let bundle = scripts.join("WindowCraft");
        write_text(&bundle.join("profile.json"), &profile.to_json())?;
        let executable = std::env::current_exe().map_err(|e| IsoError::io("current executable", e))?;
        fs::copy(&executable, bundle.join("windowcraft.exe"))
            .map_err(|e| IsoError::io(bundle.display().to_string(), e))?;
    }
    Ok(())
}

/// Metadata that must be unchanged by servicing.
fn fingerprint(wim: &Path) -> Result<Vec<(u32, String, String)>> {
    let images = WimFile::open(wim)?.images()?;
    Ok(images.into_iter().map(|image| (image.index, image.name, image.architecture)).collect())
}

fn inject_into(wim: &Path, index: u32, packages: &[drivers::DriverPackage], mount: &Path) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let before = fingerprint(wim)?;
    fs::create_dir_all(mount).map_err(|e| IsoError::io(mount.display().to_string(), e))?;
    let image = MountedImage::mount(wim, index, mount)?;
    drivers::inject(mount, packages)?;
    image.commit()?;
    if fingerprint(wim)? != before {
        return Err(IsoError::Invalid(format!("{} metadata changed during servicing", wim.display())));
    }
    Ok(())
}

/// Step two: stage configuration and optionally inject drivers.
pub fn configure(session: &mut StudioSession, work: &Path, catalog: &Catalog, context: &JobContext) -> Result<()> {
    let media = work.join("media");
    if let Some(image) = install_image(&media) {
        align_edition(session, &WimFile::open(&image)?.images()?)?;
    }
    step(context, 0.3, IsoStep::WritingAnswerFile);
    stage_files(&media, session, catalog)?;

    if session.inject_drivers {
        // Clears mounts left by any earlier crash.
        cleanup_mountpoints()?;
        step(context, 0.4, IsoStep::ExportingDrivers);
        let export = work.join("drivers");
        let packages = drivers::export_system_drivers(&export)?;
        let (boot, install) = drivers::partition_by_target(packages);
        let mount = work.join("mount");

        step(context, 0.5, IsoStep::InjectingBoot(boot.len()));
        inject_into(&media.join(r"sources\boot.wim"), BOOT_IMAGE_INDEX, &boot, &mount)?;

        // DISM cannot service an ESD, so the chosen edition becomes a WIM first.
        if media.join(r"sources\install.esd").exists() && !media.join(r"sources\install.wim").exists() {
            step(context, 0.55, IsoStep::ConvertingEsd);
            convert_esd(session, &media, catalog)?;
        }
        let image = install_image(&media)
            .filter(|path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wim")))
            .ok_or_else(|| IsoError::Invalid("no sources\\install.wim to add drivers to".to_owned()))?;
        step(context, 0.6, IsoStep::InjectingInstall(install.len()));
        inject_into(&image, session.options.image_index, &install, &mount)?;
    }
    session.step = Step::Configured;
    session.save(work)
}

fn find_oscdimg() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join(r"Microsoft\WinGet\Packages"));
    }
    if let Some(files) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(files).join(r"Windows Kits\10\Assessment and Deployment Kit\Deployment Tools"));
    }
    fn search(directory: &Path, depth: u32) -> Option<PathBuf> {
        let entries: Vec<_> = fs::read_dir(directory).ok()?.flatten().collect();
        if let Some(found) = entries.iter().find(|entry| entry.file_name().eq_ignore_ascii_case("oscdimg.exe")) {
            return Some(found.path());
        }
        if depth == 0 {
            return None;
        }
        entries.iter().filter(|entry| entry.path().is_dir()).find_map(|entry| search(&entry.path(), depth - 1))
    }
    roots.iter().find_map(|root| search(root, 5))
}

/// Dual BIOS/UEFI boot sectors. Paths are relative to the media root.
pub fn oscdimg_arguments(media: &Path, output: &Path) -> Vec<String> {
    let bios = media.join(r"boot\etfsboot.com");
    let uefi = media.join(r"efi\microsoft\boot\efisys.bin");
    vec![
        "-m".to_owned(),
        "-o".to_owned(),
        "-u2".to_owned(),
        "-udfver102".to_owned(),
        // Same volume label as a USB drive written by ISO Studio.
        "-lWIN11".to_owned(),
        format!("-bootdata:2#p0,e,b{}#pEF,e,b{}", bios.display(), uefi.display()),
        media.display().to_string(),
        output.display().to_string(),
    ]
}

/// Step three, option one: a bootable ISO.
pub fn export_iso(session: &mut StudioSession, work: &Path, output: &Path, context: &JobContext) -> Result<()> {
    let mut tool = find_oscdimg();
    let mut borrowed = None;
    if tool.is_none() {
        step(context, 0.7, IsoStep::InstallingOscdimg);
        let winget =
            windowcraft_engine::packages::winget_path().ok_or_else(|| IsoError::Invalid("winget.exe is required".to_owned()))?;
        let args = ["install", "--id", OSCDIMG_ID, "--exact", "--silent", "--accept-package-agreements", "--accept-source-agreements"];
        run_ok(&winget, &args, &context.cancel, Duration::from_secs(600))?;
        tool = find_oscdimg();
        borrowed = Some(winget);
    }
    let tool = tool.ok_or_else(|| IsoError::Invalid("oscdimg.exe was not found after installation".to_owned()))?;

    step(context, 0.75, IsoStep::BuildingIso);
    let arguments = oscdimg_arguments(&work.join("media"), output);
    let references: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let built = run_ok(&tool, &references, &context.cancel, Duration::from_secs(60 * 60));
    // Nobody asked for this tool, so it leaves with the job that needed it,
    // even after a cancelled build.
    if let Some(winget) = borrowed {
        let args = ["uninstall", "--id", OSCDIMG_ID, "--exact", "--silent", "--disable-interactivity"];
        let cancel = windowcraft_engine::jobs::CancelToken::new();
        let _ = run_ok(&winget, &args, &cancel, Duration::from_secs(120));
    }
    built?;
    session.step = Step::Exported;
    session.save(work)
}

/// An ESD cannot be split, so one above the FAT32 file size limit has to
/// become a WIM first. The export holds one image, which moves it to index 1.
fn convert_oversized_esd(session: &mut StudioSession, work: &Path, catalog: &Catalog, context: &JobContext) -> Result<()> {
    let media = work.join("media");
    let esd = media.join(r"sources\install.esd");
    if !fs::metadata(&esd).is_ok_and(|metadata| metadata.len() > SWM_PART_BYTES) {
        return Ok(());
    }
    step(context, 0.72, IsoStep::ConvertingEsd);
    convert_esd(session, &media, catalog)
}

/// Exports the chosen edition of install.esd into install.wim and removes the
/// ESD. The WIM holds that one image, so the answer file moves to index 1.
fn convert_esd(session: &mut StudioSession, media: &Path, catalog: &Catalog) -> Result<()> {
    let esd = media.join(r"sources\install.esd");
    let wim = media.join(r"sources\install.wim");
    WimFile::open(&esd)?.export_image(session.options.image_index, &WimFile::create(&wim)?)?;
    fs::remove_file(&esd).map_err(|e| IsoError::io(esd.display().to_string(), e))?;
    session.options.image_index = 1;
    stage_files(media, session, catalog)
}

/// Copies the media tree to a prepared stick, splitting a large install image
/// so the FAT32 file size limit never applies.
pub fn copy_to_usb(media: &Path, root: &Path, context: &JobContext) -> Result<()> {
    let image = media.join(r"sources\install.wim");
    let oversized = fs::metadata(&image).is_ok_and(|metadata| metadata.len() > SWM_PART_BYTES);
    // Parked beside the media tree, because a file above 4 GB cannot be
    // copied to FAT32 at all.
    let parked = media.with_file_name("install.wim.parked");

    if oversized {
        step(context, 0.75, IsoStep::SplittingWim);
        let sources = media.join("sources");
        remove_split_parts(&sources)?;
        let split_target = sources.join("install.swm");
        WimFile::open(&image)?.split(&split_target, SWM_PART_BYTES)?;
        fs::rename(&image, &parked).map_err(|e| IsoError::io(image.display().to_string(), e))?;
    }

    step(context, 0.8, IsoStep::CopyingToUsb);
    let copied = copy_tree(media, root, Some(context));

    if oversized {
        // The work tree must stay usable whatever the copy did. Parts left in
        // it would go into the next ISO next to install.wim.
        fs::rename(&parked, &image).map_err(|e| IsoError::io(image.display().to_string(), e))?;
        remove_split_parts(&media.join("sources"))?;
    }
    copied
}

/// Deletes `install.swm`, `install2.swm` and the other parts of a split image.
fn remove_split_parts(sources: &Path) -> Result<()> {
    let Ok(entries) = fs::read_dir(sources) else { return Ok(()) };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("install") && name.ends_with(".swm") {
            fs::remove_file(&path).map_err(|e| IsoError::io(path.display().to_string(), e))?;
        }
    }
    Ok(())
}

/// Step three, option two: write directly to a USB drive.
pub fn export_usb(
    session: &mut StudioSession,
    work: &Path,
    catalog: &Catalog,
    disk: u32,
    identity: &DiskIdentity,
    context: &JobContext,
) -> Result<()> {
    convert_oversized_esd(session, work, catalog, context)?;
    step(context, 0.74, IsoStep::Partitioning);
    let root = prepare_disk(disk, identity, "WIN11")?;
    copy_to_usb(&work.join("media"), &root, context)?;
    session.step = Step::Exported;
    session.save(work)
}

/// Downloads an ISO from a direct URL and checks it against a known hash.
pub fn download_iso(url: &str, destination: &Path, sha256: Option<&str>, context: &JobContext) -> Result<()> {
    block_on(download(url, destination, sha256, &context.cancel, |received, total| {
        if let Some(total) = total {
            context.progress(received as f32 / total as f32, format!("{} / {} MB", received >> 20, total >> 20));
        }
    }))
    .map_err(IsoError::Engine)?
    .map_err(IsoError::Engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().unwrap()
    }

    #[test]
    fn split_parts_are_removed_and_the_rest_of_sources_stays() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["install.swm", "INSTALL2.SWM", "install.wim", "boot.wim", "other.swm"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        remove_split_parts(dir.path()).unwrap();
        let mut left: Vec<String> =
            fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["boot.wim", "install.wim", "other.swm"]);
        remove_split_parts(&dir.path().join("missing")).unwrap();
    }

    fn media() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sources")).unwrap();
        dir
    }

    fn session() -> StudioSession {
        StudioSession::new(PathBuf::from(r"C:\Win11.iso"), AnswerOptions::default())
    }

    #[test]
    fn stages_answer_file_and_removes_pid() {
        let dir = media();
        fs::write(dir.path().join(r"sources\PID.txt"), "[PID]").unwrap();
        stage_files(dir.path(), &session(), &catalog()).unwrap();

        assert!(fs::read_to_string(dir.path().join("autounattend.xml")).unwrap().contains("/IMAGE/INDEX"));
        assert!(fs::read_to_string(dir.path().join(r"sources\ei.cfg")).unwrap().contains("Professional"));
        assert!(!dir.path().join(r"sources\PID.txt").exists());
        assert!(dir.path().join(r"sources\$OEM$\$$\Setup\Scripts\SetupComplete.cmd").exists());
    }

    #[test]
    fn missing_pid_file_is_fine() {
        let dir = media();
        stage_files(dir.path(), &session(), &catalog()).unwrap();
    }

    #[test]
    fn bundled_profile_is_staged_with_the_executable() {
        let dir = media();
        let mut session = session();
        let mut profile = Profile::new();
        profile.apps.push("7zip".to_owned());
        session.profile = Some(profile.clone());
        stage_files(dir.path(), &session, &catalog()).unwrap();

        let bundle = dir.path().join(r"sources\$OEM$\$$\Setup\Scripts\WindowCraft");
        assert_eq!(Profile::parse(&fs::read_to_string(bundle.join("profile.json")).unwrap()).unwrap(), profile);
        assert!(bundle.join("windowcraft.exe").exists());
        let answer = fs::read_to_string(dir.path().join("autounattend.xml")).unwrap();
        assert!(answer.contains("--config"), "the profile must run at first logon");
    }

    #[test]
    fn edition_selection_sets_the_answer_file_architecture() {
        let mut options = AnswerOptions::default();
        let image = ImageInfo {
            index: 3,
            name: "Windows 11 Pro".to_owned(),
            edition: "Professional".to_owned(),
            architecture: "ARM64".to_owned(),
            build: "26100".to_owned(),
        };
        select_edition(&mut options, &image);
        assert_eq!((options.image_index, options.architecture.as_str()), (3, "arm64"));
        select_edition(&mut options, &ImageInfo { architecture: "x64".to_owned(), ..image });
        assert_eq!(options.architecture, "amd64");
    }

    #[test]
    fn session_survives_interruption() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = session();
        session.step = Step::Configured;
        session.inject_drivers = true;
        session.save(dir.path()).unwrap();
        assert_eq!(StudioSession::resume(dir.path()), Some(session));

        fs::write(dir.path().join("session.json"), "{corrupt").unwrap();
        assert_eq!(StudioSession::resume(dir.path()), None, "corrupt state starts fresh");
    }

    #[test]
    fn media_without_an_install_image_is_rejected() {
        let dir = media();
        assert!(validate_media(dir.path()).is_err());
    }

    #[test]
    fn edition_defaults_to_pro() {
        let image = |index, edition: &str| ImageInfo {
            index,
            name: String::new(),
            edition: edition.to_owned(),
            architecture: "x64".to_owned(),
            build: "26100".to_owned(),
        };
        let images = [image(1, "Core"), image(6, "Professional")];
        assert_eq!(default_edition(&images).unwrap().index, 6);
        assert_eq!(default_edition(&images[..1]).unwrap().index, 1);
        assert!(default_edition(&[]).is_none());
    }

    #[test]
    fn iso_arguments_describe_dual_boot() {
        let arguments = oscdimg_arguments(Path::new(r"C:\w\media"), Path::new(r"C:\out.iso"));
        let boot = arguments.iter().find(|argument| argument.starts_with("-bootdata")).unwrap();
        assert!(arguments.contains(&"-lWIN11".to_owned()));
        assert!(boot.starts_with("-bootdata:2#p0,e,b"));
        assert!(boot.contains("etfsboot.com"));
        assert!(boot.contains("#pEF,e,b"));
        assert!(boot.contains("efisys.bin"));
        assert_eq!(arguments.last().unwrap(), r"C:\out.iso");
    }

    #[test]
    fn a_small_image_is_copied_without_splitting_and_stays_in_the_work_tree() {
        let work = tempfile::tempdir().unwrap();
        let media = work.path().join("media");
        fs::create_dir_all(media.join("sources")).unwrap();
        fs::write(media.join(r"sources\install.wim"), b"wim").unwrap();
        fs::write(media.join("setup.exe"), b"exe").unwrap();
        let usb = tempfile::tempdir().unwrap();

        let job = windowcraft_engine::jobs::spawn("usb", {
            let (media, root) = (media.clone(), usb.path().to_path_buf());
            move |context| {
                copy_to_usb(&media, &root, context).map_err(|e| windowcraft_engine::EngineError::Invalid(e.to_string()))?;
                Ok(String::new())
            }
        });
        job.join();

        assert_eq!(fs::read(usb.path().join(r"sources\install.wim")).unwrap(), b"wim");
        assert!(media.join(r"sources\install.wim").exists());
        assert!(!usb.path().join("install.wim.parked").exists());
    }

    #[test]
    fn copy_clears_read_only_attributes() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let file = source.path().join("f");
        fs::write(&file, b"x").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();

        copy_tree(source.path(), target.path(), None).unwrap();
        assert!(!fs::metadata(target.path().join("f")).unwrap().permissions().readonly());
    }

    fn image(index: u32, edition: &str) -> ImageInfo {
        ImageInfo {
            index,
            name: format!("Windows 11 {edition}"),
            edition: edition.to_owned(),
            architecture: "x64".to_owned(),
            build: "26100".to_owned(),
        }
    }

    #[test]
    fn the_chosen_edition_is_found_again_after_an_esd_became_a_one_image_wim() {
        let full = [image(1, "Core"), image(6, "Professional")];
        let mut chosen = session();
        align_edition(&mut chosen, &full).unwrap();
        assert_eq!(chosen.options.image_index, 6, "the index stays when it matches");

        align_edition(&mut chosen, &[image(1, "Professional")]).unwrap();
        assert_eq!(chosen.options.image_index, 1);

        let mut other = session();
        other.options.edition_id = "Education".to_owned();
        assert!(align_edition(&mut other, &[image(1, "Professional")]).is_err());
    }

    #[test]
    fn an_esd_becomes_a_wim_with_the_chosen_image_at_index_1() {
        let dir = media();
        let content = tempfile::tempdir().unwrap();
        fs::write(content.path().join("setup.exe"), b"x").unwrap();
        let esd = dir.path().join(r"sources\install.esd");
        {
            let file = WimFile::create(&esd).unwrap();
            for _ in 0..2 {
                if !crate::wim::tests::capture(&file, content.path()) {
                    eprintln!("skipped: capture needs backup privileges here");
                    return;
                }
            }
        }
        let mut chosen = session();
        chosen.options.image_index = 2;
        convert_esd(&mut chosen, dir.path(), &catalog()).unwrap();

        assert!(!esd.exists());
        let wim = dir.path().join(r"sources\install.wim");
        assert_eq!(WimFile::open(&wim).unwrap().image_count().unwrap(), 1);
        assert_eq!(chosen.options.image_index, 1);
        let answer = fs::read_to_string(dir.path().join("autounattend.xml")).unwrap();
        assert!(answer.contains("<Value>1</Value>"), "{answer}");
    }

    #[test]
    fn a_second_iso_leaves_no_file_of_the_first() {
        let first = media();
        fs::write(first.path().join(r"sources\install.wim"), b"old wim").unwrap();
        fs::write(first.path().join("only-in-first.txt"), b"x").unwrap();
        let second = media();
        fs::write(second.path().join(r"sources\install.esd"), b"new esd").unwrap();

        let work = tempfile::tempdir().unwrap();
        let target = work.path().join("media");
        copy_media(first.path(), &target, None).unwrap();
        // Media from an ISO keeps its read-only flag in older work folders.
        let old = target.join(r"sources\install.wim");
        let mut permissions = fs::metadata(&old).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&old, permissions).unwrap();

        copy_media(second.path(), &target, None).unwrap();
        assert!(!old.exists());
        assert!(!target.join("only-in-first.txt").exists());
        assert_eq!(install_image(&target), Some(target.join(r"sources\install.esd")));
    }
}
