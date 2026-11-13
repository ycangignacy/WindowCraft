<!-- Made by WindowCraft Dev Team -->
# WindowCraft VM Test Plan

Checklist for the first live run of system-changing features inside a disposable Windows 11 virtual machine (VirtualBox or VMware Workstation/Player, since Windows 11 Home hosts do not include Hyper-V).

> [!WARNING]
> Run every step below inside the guest VM only. Several actions (AppX package removal, application uninstallation, BitLocker decryption, DISM feature enabling, and USB disk erasure) cannot be rolled back by Time Machine. Always take a clean hypervisor snapshot before starting.

---

## Preparation

### 1. Build and copy the binaries
- [ ] On the host, build the release binaries:
  ```powershell
  cargo build --release -p windowcraft-app -p windowcraft-cli
  ```
- [ ] Copy `target\release\windowcraft.exe` and `target\release\windowcraft-cli.exe` into the VM (for example to `C:\Test\windowcraft.exe` and `C:\Test\windowcraft-cli.exe`). The binaries are self-contained and embed the JSON catalog at compile time.
- [ ] Copy a stock Windows 11 `.iso` file into the VM (for example `C:\Test\Win11.iso`) so the ISO Studio and USB write sections have a source image.

### 2. Take a baseline VM snapshot
- [ ] Boot the Windows 11 VM to the desktop, let initial background tasks settle, and take a hypervisor snapshot named **`Pre-WindowCraft-Clean`** in VirtualBox (`Machine -> Take Snapshot`) or VMware (`VM -> Snapshot -> Take Snapshot`).

### 3. Verify elevation paths
- [ ] Launch `C:\Test\windowcraft.exe` as a standard (non-elevated) user.
- [ ] Verify the top-left sidebar displays an amber **`Standard user`** badge.
- [ ] Click the **`Standard user`** badge (or the **`Restart as Administrator`** button on the banner if shown) and accept the UAC prompt (`ShellExecuteW` with `"runas"`).
- [ ] Confirm the new window displays a green **`Administrator`** badge in the sidebar and closes the unelevated instance.
- [ ] Open an unelevated PowerShell window and run:
  ```powershell
  C:\Test\windowcraft-cli.exe --preset minimal --run
  ```
  Confirm it exits with code `1` and prints:
  `error: this command changes system settings: run it from an elevated terminal`
- [ ] Open an **elevated** PowerShell (`Run as Administrator`) and keep it open for all CLI runs and verification commands below.

### 4. Local storage and log paths
WindowCraft resolves its data directory via `LOCALAPPDATA` (`%LOCALAPPDATA%\WindowCraft`, falling back to `%TEMP%\WindowCraft` if `LOCALAPPDATA` is unset):

| File / Directory | Path | Purpose |
| :--- | :--- | :--- |
| Snapshot database | `%LOCALAPPDATA%\WindowCraft\timemachine.redb` | `redb` database holding `sessions`, `changes`, and `applied` tables. If corrupted, renamed to `timemachine.corrupt-<unix_timestamp>`. |
| Job log | `%LOCALAPPDATA%\WindowCraft\windowcraft.log` | Append-only job execution log. Rolls over to `windowcraft.log.old` once it exceeds 1 MiB. |
| Exported profile | `%LOCALAPPDATA%\WindowCraft\profile.json` | Written when clicking **Export Profile** in the GUI. |
| Environment report | `%LOCALAPPDATA%\WindowCraft\environment-report.json` | Written when clicking **Export Report** in the GUI (or to a custom path via `--export-report <PATH>`). |
| ISO work directory | `%LOCALAPPDATA%\WindowCraft\iso\` | Staging area for ISO Studio (`session.json`, `media\`, `drivers\`, `mount\`, and downloaded `Win11.iso`). |

- [ ] Before running any tweaks, confirm `%LOCALAPPDATA%\WindowCraft` is created on first launch.

---

## 1. Tweaks and Rollback through Time Machine

### Apply tweaks
- [ ] In the elevated WindowCraft GUI, open **`Tweaks & Privacy`** (`Essential` tab).
- [ ] Select **`Activity History - Disable`** (`id: "activity"`). Click the inspect icon next to the row to open the **X-Ray Inspector** drawer and verify it lists three `REG_DWORD` writes under `HKLM\SOFTWARE\Policies\Microsoft\Windows\System`:
  - `EnableActivityFeed` = `1`
  - `PublishUserActivities` = `0`
  - `UploadUserActivities` = `0`
- [ ] Switch to the **`Preferences`** tab:
  - Toggle **`Dark Theme for Windows`** (`id: "dark-mode"`) **On** (`HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize`: `AppsUseLightTheme = 0`, `SystemUsesLightTheme = 0`).
  - Toggle **`File Explorer File Extensions`** (`id: "show-ext"`) **On** (`HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced`: `HideFileExt = 0`).
  - On the segmented control **`Multiplane Overlay`** (`id: "multiplane-overlay"`), select **`Disabled (Compatibility)`** (`HKLM\SOFTWARE\Microsoft\Windows\Dwm`: `OverlayTestMode = 5`).
- [ ] Click **`Run Tweaks`** in the bottom floating dock (or apply the `activity` tweak from the X-Ray Inspector with **`Apply This Tweak`**).
  - *CLI alternative:* save `{"version":1,"tweaks":["activity"]}` to `C:\Test\tweak.json` and run `C:\Test\windowcraft-cli.exe --config C:\Test\tweak.json --run`.

### Verify with independent commands
- [ ] Run in elevated PowerShell / Command Prompt:
  ```cmd
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v EnableActivityFeed
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v PublishUserActivities
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v UploadUserActivities
  reg query "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize" /v AppsUseLightTheme
  reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" /v HideFileExt
  reg query "HKLM\SOFTWARE\Microsoft\Windows\Dwm" /v OverlayTestMode
  ```
- [ ] **Expected result:** `EnableActivityFeed` is `0x1`, `PublishUserActivities` is `0x0`, `UploadUserActivities` is `0x0`, `AppsUseLightTheme` is `0x0`, `HideFileExt` is `0x0`, and `OverlayTestMode` is `0x5`.
- [ ] Verify `%LOCALAPPDATA%\WindowCraft\timemachine.redb` and `%LOCALAPPDATA%\WindowCraft\windowcraft.log` exist and contain the completed session entry.

### Verify rollback
- [ ] Open the **`Time Machine`** module in the sidebar.
- [ ] Click the latest session row to expand its recorded changes. Confirm each change shows the registry path, `before -> after`, and current `live` state.
- [ ] Click the per-change undo button on `OverlayTestMode` (`RollbackChange`) and verify `reg query "HKLM\SOFTWARE\Microsoft\Windows\Dwm" /v OverlayTestMode` reports that the value was restored to its prior state (or removed if it did not exist before).
- [ ] Click **`Roll Back`** on the session row and confirm **`Roll Back`** in the modal dialog.
- [ ] Re-run the `reg query` commands above.
- [ ] **Expected result:** All registry values written during the session are restored to their exact pre-session values (or deleted if `before` was `Absent`), and the session badge in Time Machine changes to **`Rolled back`**. If a session includes operations that could not be reversed, Time Machine lists those items and marks the session **`Partly rolled back`**.

---

## 2. Drift Detection

### Create a baseline and induce drift
- [ ] Apply **`Activity History - Disable`** (`id: "activity"`) again from **`Tweaks & Privacy`** -> **`Essential`** -> **`Run Tweaks`**.
- [ ] Confirm no drift is reported initially:
  ```powershell
  C:\Test\windowcraft-cli.exe --check-drift
  ```
  **Expected result:** Prints `No drift detected.` and exits with code `0`.
- [ ] Simulate an external change (such as a Windows Update resetting a policy value) by overwriting `EnableActivityFeed`:
  ```cmd
  reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v EnableActivityFeed /t REG_DWORD /d 0 /f
  ```

### Verify drift detection (CLI and GUI)
- [ ] Run the CLI drift check:
  ```powershell
  C:\Test\windowcraft-cli.exe --check-drift
  ```
  **Expected result:** Prints a summary line saying one applied tweak was reset, then the tweak name, then a line for `EnableActivityFeed` showing the expected and found values. Exits with code `0`, because finding drift is a report and no error.
- [ ] In the GUI, navigate to **`Time Machine`** (or click **`Scan`**).
- [ ] **Expected result:** An amber drift banner appears at the top (`1 applied tweak was reset by Windows Update or another tool`) with a **`Re-apply`** button, and the **`Configuration Drift`** section lists `Activity History - Disable`.

### Verify drift repair and baseline preservation
- [ ] Repair the drifted value either by clicking **`Re-apply`** in the GUI or by running:
  ```powershell
  C:\Test\windowcraft-cli.exe --check-drift --fix
  ```
- [ ] Verify the repaired registry value:
  ```cmd
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v EnableActivityFeed
  ```
  **Expected result:** `EnableActivityFeed` is back to `0x1`, and `windowcraft-cli.exe --check-drift` reports `No drift detected.` and exits with code `0`.
- [ ] In **`Time Machine`**, roll back the **original** session that first applied `Activity History - Disable`.
- [ ] Run `reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\System" /v EnableActivityFeed`.
- [ ] **Expected result:** Because `repair_drift` preserves the original `baseline_session`, rolling back the original session restores the true pre-tweak baseline.

---

## 3. Services

### Apply service startup changes
- [ ] Record the initial startup types of `DiagTrack` and `MapsBroker`:
  ```cmd
  sc qc DiagTrack
  sc qc MapsBroker
  ```
- [ ] In **`Tweaks & Privacy`** -> **`Essential`**, select **`Services - Set to Manual`** (`id: "services"`, which sets `CscService` -> `Disabled`, `DiagTrack` -> `Disabled`, `MapsBroker` -> `Manual`, `StorSvc` -> `Manual`, `SharedAccess` -> `Disabled`) and click **`Run Tweaks`**.
- [ ] Also open **`System & Gaming Inspector`** -> **`Autoruns`** -> **`Services`**, locate a non-protected service such as `Fax` or `MapsBroker`, and toggle its switch between `Automatic`/`Manual` and `Disabled`.

### Verify with independent commands
- [ ] Run in elevated Command Prompt:
  ```cmd
  sc qc DiagTrack
  sc qc MapsBroker
  sc qc SharedAccess
  ```
- [ ] **Expected result:**
  - `DiagTrack` shows `START_TYPE : 4  DISABLED`
  - `MapsBroker` shows `START_TYPE : 3  DEMAND_START`
  - `SharedAccess` shows `START_TYPE : 4  DISABLED`

### Verify protected-services guard
WindowCraft hard-blocks disabling seven critical services (`RpcSs`, `DcomLaunch`, `LSM`, `BFE`, `mpssvc`, `WinDefend`, `ProfSvc`):
- [ ] In **`System & Gaming Inspector`** -> **`Autoruns`** -> **`Services`**, locate `WinDefend` or `BFE` (`Base Filtering Engine`). Confirm the row displays a **`Protected`** badge instead of a toggle switch.
- [ ] Create `C:\Test\protected-service.json`:
  ```json
  {
    "version": 1,
    "services": [
      { "name": "WinDefend", "startup": "Disabled", "original": "Automatic" }
    ]
  }
  ```
- [ ] Run:
  ```powershell
  C:\Test\windowcraft-cli.exe --config C:\Test\protected-service.json --run
  ```
- [ ] **Expected result:** The command exits with code `1`, refuses the change with `service 'WinDefend' is protected and cannot be disabled`, and `sc qc WinDefend` confirms `WinDefend` was not modified.

### Verify rollback
- [ ] Open **`Time Machine`**, find the **`Run Tweaks`** session that applied `Services - Set to Manual`, and click **`Roll Back`**.
- [ ] Run `sc qc DiagTrack` and `sc qc MapsBroker`.
- [ ] **Expected result:** Each service's `START_TYPE` matches the initial state recorded before the test.

---

## 4. AppX Removal

> [!IMPORTANT]
> **Cannot be rolled back automatically:** Removed AppX packages and deprovisioned system apps are not restored by Time Machine rollback. Reinstallation requires Microsoft Store / `winget` network access or restoring the VM snapshot.

### Remove an AppX package
- [ ] Verify a candidate package is currently present (for example `Microsoft.WindowsFeedbackHub` or `Clipchamp.Clipchamp`):
  ```powershell
  Get-AppxPackage -AllUsers *WindowsFeedbackHub*
  Get-AppxProvisionedPackage -Online | Where-Object DisplayName -like "*WindowsFeedbackHub*"
  ```
- [ ] In **`Tweaks & Privacy`** -> **`AppX Packages`**, select **`Feedback Hub`** (`Microsoft.WindowsFeedbackHub`, `id: "microsoft-windows-feedback-hub"`). (Alternatively, click **`Default Removals`** to select the 18 default consumer bloat packages.)
- [ ] Click **`Remove`** in the bottom floating dock.
  - *CLI alternative:* save `{"version":1,"appx_remove":["microsoft-windows-feedback-hub"]}` to `C:\Test\appx.json` and run `C:\Test\windowcraft-cli.exe --config C:\Test\appx.json --run`.

### Verify with independent commands
- [ ] Run in elevated PowerShell:
  ```powershell
  Get-AppxPackage -AllUsers *WindowsFeedbackHub*
  Get-AppxProvisionedPackage -Online | Where-Object DisplayName -like "*WindowsFeedbackHub*"
  ```
- [ ] **Expected result:** Both commands return no output (`RemovePackageWithOptionsAsync` with `RemoveForAllUsers` and `DeprovisionPackageForAllUsersAsync` succeeded).

### Verify rollback limitation and manual reinstall path
- [ ] Open **`Time Machine`** and inspect the session. Confirm AppX removal does not record a reversible registry/service snapshot.
- [ ] (Optional, if the VM has network and `winget`) In **`Tweaks & Privacy`** -> **`AppX Packages`**, select **`Feedback Hub`** and click **`Install`** in the dock to verify it invokes `winget install --id 9NBLGGH4R32N`.

---

## 5. DISM Features

> [!NOTE]
> Rollback behaviour: Time Machine records each feature with its state before WindowCraft enabled it. A rollback disables the features that were off before and leaves those that were on, and it reverts registry entries attached to the feature (for example `EnablePeriodicBackup` on `reg-backup`, whose rollback also deletes the `AutoRegBackup` task). Parent features that DISM enabled along with a requested one stay on.

### Enable a Windows Optional Feature
- [ ] Check the current state of `Microsoft-Windows-Subsystem-Linux` and `VirtualMachinePlatform` (or `TelnetClient` / `SimpleTCP` if testing a lightweight feature):
  ```cmd
  dism /online /get-features /format:table | findstr /i "Subsystem-Linux VirtualMachinePlatform"
  ```
- [ ] In **`System Config & Fixes`** -> **`Windows Optional Features`**, check **`Windows Subsystem for Linux (WSL) - Enable`** (`id: "wsl"`, which runs `dism.exe /Online /Enable-Feature /FeatureName:<name> /All /NoRestart` for `VirtualMachinePlatform` and `Microsoft-Windows-Subsystem-Linux`).
- [ ] Also check **`Registry Backup Task (RegIdleBackup) - Enable`** (`id: "reg-backup"`, which writes `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Configuration Manager\EnablePeriodicBackup = 1`).
- [ ] Click **`Install Features`** in the bottom floating dock.
  - *CLI alternative:* save `{"version":1,"features":["wsl","reg-backup"]}` to `C:\Test\features.json` and run `C:\Test\windowcraft-cli.exe --config C:\Test\features.json --run`.

### Verify with independent commands
- [ ] Run in elevated Command Prompt:
  ```cmd
  dism /online /get-features /format:table | findstr /i "Subsystem-Linux VirtualMachinePlatform"
  reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Configuration Manager" /v EnablePeriodicBackup
  ```
- [ ] **Expected result:** Both DISM features show state **`Enabled`** or **`Enable Pending`** (exit code `3010` / reboot required is treated as success by the engine), and `EnablePeriodicBackup` is `0x1`.

### Verify rollback scope
- [ ] Roll back the feature session in **`Time Machine`**.
- [ ] Verify `reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Configuration Manager" /v EnablePeriodicBackup` reverts `EnablePeriodicBackup`.
- [ ] Verify that rolling back `reg-backup` deletes the `AutoRegBackup` scheduled task:
  ```cmd
  schtasks /query /tn "AutoRegBackup"
  ```
  **Expected result:** `ERROR: The system cannot find the file specified.`
- [ ] Verify `Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Windows-Subsystem-Linux` reports `Disabled` or `DisablePending`, and that a feature which was on before the session is still on.
- [ ] Verify the session in Time Machine displays **`Rolled back`**.

---

## 6. Windows Update Policy Modes

### Mode A: `Security / Recommended` (`id: "security"`)
- [ ] Open the **`Windows Updates`** module in the sidebar.
- [ ] On the **`Security / Recommended`** card, click **`Apply`**.
  - *CLI alternative:* save `{"version":1,"update_mode":"security"}` to `C:\Test\wu-sec.json` and run `C:\Test\windowcraft-cli.exe --config C:\Test\wu-sec.json --run`.
- [ ] Verify with independent commands:
  ```cmd
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate"
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU"
  reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\DriverSearching" /v DontSearchWindowsUpdate
  ```
- [ ] **Expected result:**
  - `DeferFeatureUpdates` = `0x1`, `DeferFeatureUpdatesPeriodInDays` = `0x16d` (`365`)
  - `DeferQualityUpdates` = `0x1`, `DeferQualityUpdatesPeriodInDays` = `0x4` (`4`)
  - `ExcludeWUDriversInQualityUpdate` = `0x1`
  - `AU\NoAutoRebootWithLoggedOnUsers` = `0x1`
  - `DontSearchWindowsUpdate` = `0x1`
  - The **`Security / Recommended`** card displays the **`Active`** badge.

### Mode B: `Disable ALL Updates` (`id: "disabled"`)
- [ ] First verify the unattended CLI safety guard. Save `{"version":1,"update_mode":"disabled"}` to `C:\Test\wu-dis.json` and run:
  ```powershell
  C:\Test\windowcraft-cli.exe --config C:\Test\wu-dis.json --run
  ```
  **Expected result:** Exits with code `1` and fails with `operation 'Disable ALL Updates' requires explicit user confirmation` because unattended profiles do not pass `confirm_destructive = true`.
- [ ] In the GUI **`Windows Updates`** module, click **`Apply`** on **`Disable ALL Updates`**.
- [ ] Verify the confirmation modal **`Disable all Windows Updates?`** appears; click **`Disable Updates`**.
- [ ] Verify with independent commands:
  ```cmd
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU" /v NoAutoUpdate
  sc qc wuauserv
  sc qc BITS
  sc qc UsoSvc
  schtasks /query /tn "\Microsoft\Windows\WindowsUpdate\"
  ```
- [ ] **Expected result:**
  - `NoAutoUpdate` is `0x1`.
  - `wuauserv`, `BITS`, and `UsoSvc` show `START_TYPE : 4  DISABLED` and are stopped.
  - Scheduled tasks under `\Microsoft\Windows\WindowsUpdate\` (and `InstallService`, `UpdateOrchestrator`, `UpdateAssistant`, `WaaSMedic`, `\Microsoft\WindowsUpdate`) show status `Disabled`.
  - `%SystemRoot%\SoftwareDistribution` contents were purged.

### Mode C: `Default` (`id: "default"`) and Time Machine Rollback
- [ ] In **`Time Machine`**, roll back the **`Update Mode: Disable ALL Updates`** session (or click **`Apply`** on the **`Default`** card in **`Windows Updates`**).
- [ ] Re-run the `reg query`, `sc qc`, and `schtasks` commands above.
- [ ] **Expected result:**
  - Selecting **`Default`** removes the policy values, sets `BITS` and `wuauserv` to `DEMAND_START` (`Manual`), sets `UsoSvc` to `AUTO_START` (`Automatic`), and re-enables the tasks in the six Windows Update task folders.
  - Rolling back via **`Time Machine`** restores the exact prior session state (for example returning from `Disable ALL Updates` to `Security / Recommended`).

---

## 7. BitLocker

> [!CAUTION]
> **Cannot be rolled back:** BitLocker decryption (`disable-bit-locker`) is marked `irreversible: true` in the catalog. Once decryption starts via WMI `Win32_EncryptableVolume.Decrypt`, Time Machine cannot re-encrypt the volume.

### Trigger BitLocker decryption
- [ ] Check the guest OS volume's current encryption state:
  ```cmd
  manage-bde -status C:
  ```
- [ ] In **`Tweaks & Privacy`** -> **`Essential`**, select **`BitLocker - Disable`** (`id: "disable-bit-locker"`).
- [ ] Click **`Run Tweaks`**.
- [ ] Verify the confirmation modal **`This selection cannot be undone`** (`BitLocker - Disable has no automatic undo. Continue with the queued tweaks?`) appears, and click **`Continue`**.

### Verify with independent commands
- [ ] Run in elevated Command Prompt:
  ```cmd
  manage-bde -status C:
  ```
- [ ] **Expected result:**
  - If `C:` was encrypted, `Conversion Status` changes to **`Decryption in Progress`** or **`Fully Decrypted`**, and `Percentage Encrypted` decreases toward `0.0%`.
  - If `C:` was already unencrypted (common on default VM disks), `Conversion Status` stays **`Fully Decrypted`**.
- [ ] Open the **X-Ray Inspector** on **`BitLocker - Disable`** and verify it displays the amber warning **`This change cannot be undone automatically`** with no **`Undo This Tweak`** button.

---

## 8. ISO Studio Pipeline

### Extract and configure a Windows 11 ISO
- [ ] Open **`ISO Studio`** in the sidebar.
- [ ] Under **`1. Source`**, enter the path to the local Windows 11 ISO (for example `C:\Test\Win11.iso`) and click **`Open`**.
- [ ] Wait for extraction to finish and verify `%LOCALAPPDATA%\WindowCraft\iso\session.json` exists with `"step": "Extracted"` and `%LOCALAPPDATA%\WindowCraft\iso\media\` contains the extracted ISO file tree.
- [ ] Under **`2. Edition`**, verify the editions parsed from `sources\install.wim` (or `install.esd` via `wimgapi.dll`) are listed and **`Windows 11 Pro`** (`Professional`) is selected by default if present.
- [ ] Under **`3. Configuration`**, enable:
  - **`Bypass hardware checks`** (TPM, Secure Boot, RAM, CPU, Storage via `LabConfig`)
  - **`Local account setup`** (sets `BypassNRO = 1`)
  - **`Dark mode by default`**
  - **`Disable device encryption`** (`PreventDeviceEncryption = 1`)
  - **`Pause updates during OOBE`** (`NoAutoUpdate = 1` during setup, removed at the end of `SetupComplete.cmd`)
  - **`Bundle selected applications`** (first select at least one app in **`Software & Apps`** or one tweak in **`Tweaks & Privacy`** so the profile is non-empty)
  - (Optional) **`Inject system drivers`** to export third-party host/VM drivers into `%LOCALAPPDATA%\WindowCraft\iso\drivers` and inject them into `boot.wim` (index 2, `SCSIAdapter`/`HDC` classes) and `install.wim` (selected edition index).

### Build the customized ISO
- [ ] Under **`4. Export`**, enter `C:\Test\Win11-Custom.iso` as the output path and click **`Build ISO`**.

### Verify staged artifacts and output ISO with independent commands
- [ ] Inspect the generated files in `%LOCALAPPDATA%\WindowCraft\iso\media\`:
  ```powershell
  Get-Item "$env:LOCALAPPDATA\WindowCraft\iso\media\autounattend.xml"
  Get-Content "$env:LOCALAPPDATA\WindowCraft\iso\media\sources\ei.cfg"
  Test-Path "$env:LOCALAPPDATA\WindowCraft\iso\media\sources\PID.txt"
  Get-Item "$env:LOCALAPPDATA\WindowCraft\iso\media\sources\`$OEM`$\`$`$\Setup\Scripts\SetupComplete.cmd"
  Get-ChildItem "$env:LOCALAPPDATA\WindowCraft\iso\media\sources\`$OEM`$\`$`$\Setup\Scripts\WindowCraft"
  ```
- [ ] **Expected result:**
  - `autounattend.xml` exists at the media root, contains `BypassTPMCheck`, `BypassSecureBootCheck`, `BypassRAMCheck`, `BypassCPUCheck`, `BypassStorageCheck`, `BypassNRO`, `PreventDeviceEncryption`, `NoAutoUpdate`, `<Key>/IMAGE/INDEX</Key>`, and `<FirstLogonCommands>`, and **does not** contain `DiskConfiguration` or `WillWipeDisk` (so Windows Setup always prompts for disk selection interactively).
  - `sources\ei.cfg` contains `[EditionID]` followed by the selected edition ID and `[Channel] Retail`.
  - `sources\PID.txt` returns `False` (deleted if present).
  - `SetupComplete.cmd` exists, mounts `C:\Users\Default\NTUSER.DAT` to `HKLM\WC_DEFAULT`, writes the privacy/dark-mode baseline, deprovisions default bloat AppX packages, disables telemetry scheduled tasks, and deletes `NoAutoUpdate` at completion.
  - `sources\$OEM$\$$\Setup\Scripts\WindowCraft\` contains both `profile.json` and a copy of `windowcraft.exe`.
- [ ] If **`Inject system drivers`** was enabled, verify no stale WIM mounts remain:
  ```cmd
  dism /Get-MountedWimInfo
  ```
  **Expected result:** No lingering mounts under `%LOCALAPPDATA%\WindowCraft\iso\mount`.
- [ ] Mount the built `C:\Test\Win11-Custom.iso` in Windows to confirm the UDF/ISO9660 + El Torito image is valid:
  ```powershell
  $m = Mount-DiskImage -ImagePath C:\Test\Win11-Custom.iso -PassThru
  Get-Volume -DiskImage $m
  Dismount-DiskImage -ImagePath C:\Test\Win11-Custom.iso
  ```
- [ ] Clear "ISO Studio work files" in the Cleaner while the ISO is open. The cleaner skips them and says so (KB-5, fixed in 1.0.0).

---

## 9. USB Write

> [!CAUTION]
> **Cannot be rolled back:** Writing to a USB drive wipes all partitions on the target disk, converts it to GPT, and formats a new FAT32 partition labeled `WIN11`. Attach only a throwaway virtual USB disk to the VM.

### Attach a throwaway virtual USB drive to the VM
- [ ] Create a blank 16 GB virtual disk (`VDI` or `VMDK`) on the host and attach it to the VM's **USB controller** (in VirtualBox: attach via USB mass storage / `VBoxManage storageattach ... --type hdd` on a USB controller or pass through a throwaway 16 GB USB flash drive; in VMware: attach a virtual disk or loopback USB drive with bus type USB).
- [ ] Inside the VM, verify Windows sees the disk on the `USB` bus:
  ```powershell
  Get-Disk | Select-Object Number, FriendlyName, BusType, IsSystem, Size, PartitionStyle
  ```

### Verify the USB safety guard
WindowCraft's `usb::assess` and `usb::prepare_disk` enforce four independent checks before any destructive disk I/O:
1. **Bus check:** Rejects any disk whose `STORAGE_BUS_TYPE` is not `BusTypeUsb` (`SATA`, `NVMe`, `SCSI`, `SAS`, `Virtual`, and `Other` are excluded).
2. **System disk check:** Rejects any disk where `is_system` is true or whose mounted drive letters include `%SystemDrive%` (`C:`).
3. **Shape check:** Rejects disks smaller than **8 GiB**, larger than **2048 GiB**, or having more than **8 partitions**.
4. **Pre-write re-verification:** Immediately before locking volumes and zeroing partition signatures, `prepare_disk` re-queries `\\.\PhysicalDrive<N>`, re-runs `assess`, and confirms `disk.model == confirmed_model`.

- [ ] In **`ISO Studio`** -> **`4. Export`**, click **`Find Drives`**.
- [ ] **Expected result:**
  - The VM's primary OS disk (`Disk 0` on `SATA`/`NVMe`/`SCSI` carrying `C:`) is **never** listed.
  - Only the attached virtual USB disk (`8 GiB`–`2048 GiB`, `BusType = USB`) appears in the drive list.
- [ ] (Optional boundary check) Create >8 small partitions on the virtual USB disk in Disk Management and click **`Find Drives`** again; confirm the drive is excluded until partitions are removed.

### Erase and write the virtual USB drive
- [ ] Click **`Erase and Write`** next to the virtual USB drive.
- [ ] Verify the confirmation modal **`Erase disk <N>?`** appears, displays the disk number and model string, and warns that all partitions will be wiped. Click **`Erase and Write`**.
- [ ] Verify job cancellation safety: closing the window during an active job prompts for confirmation, and quitting anyway cancels the job.

### Verify with independent commands
- [ ] Once the job completes, inspect the USB disk layout and contents in elevated PowerShell:
  ```powershell
  Get-Disk | Where-Object BusType -eq 'USB' | Select-Object Number, PartitionStyle, NumberOfPartitions
  Get-Partition -DiskNumber <N> | Select-Object PartitionNumber, DriveLetter, Size, Offset
  Get-Volume -DriveLetter <U> | Select-Object FileSystem, FileSystemLabel, Size
  Get-ChildItem "<U>:\"
  Get-ChildItem "<U>:\sources\install*"
  ```
  *(Replace `<N>` with the USB disk number and `<U>` with its assigned drive letter.)*
- [ ] **Expected result:**
  - `PartitionStyle` is `GPT` with `1` partition starting at a 1 MiB offset and capped at 32 GiB maximum.
  - `FileSystem` is `FAT32` and `FileSystemLabel` is `WIN11`.
  - `<U>:\` contains `autounattend.xml`, `setup.exe`, `sources\boot.wim`, `sources\ei.cfg`, and `sources\$OEM$\$$\Setup\Scripts\SetupComplete.cmd`.
  - If `sources\install.wim` in the staging directory exceeded **3800 MiB** (`SWM_PART_BYTES`), `<U>:\sources\` contains split `.swm` parts (`install.swm`, `install2.swm`, ...) instead of `install.wim` so every file fits within the 4 GiB FAT32 limit, while `%LOCALAPPDATA%\WindowCraft\iso\media\sources\install.wim` remains intact for future ISO builds.

---

## 10. First-Logon Cleanup

### Verify OEM setup script execution and cleanup
- [ ] Boot a guest VM using the customized ISO created in Section 8 (`C:\Test\Win11-Custom.iso`).
- [ ] Allow Windows Setup to finish installation and boot into OOBE.
- [ ] Complete local account creation.
- [ ] Upon initial desktop display, verify that `SetupComplete.cmd` ran before the user session:
  ```cmd
  reg query "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU" /v NoAutoUpdate
  ```
  **Expected result:** `NoAutoUpdate` is deleted at the conclusion of `SetupComplete.cmd`.
- [ ] Verify that default user modifications were applied to `C:\Users\Default\NTUSER.DAT` during setup.
- [ ] Verify that provisioned bloat AppX packages were removed for the newly created user profile.
- [ ] Verify the first-logon commands applied the bundled profile, then removed WindowCraft: `%WINDIR%\Setup\Scripts\WindowCraft` and `%LOCALAPPDATA%\WindowCraft` no longer exist.

---

## 11. Installer Upgrade

### Verify upgrade preserves Time Machine database and settings
- [ ] Build a second copy of the installers with a higher version in `Cargo.toml`. An upgrade needs a higher version, so two 0.1.0 builds do not test it.
- [ ] Install the older version through `setup.exe` or the MSI, then apply tweaks so `%LOCALAPPDATA%\WindowCraft\timemachine.redb` holds sessions.
- [ ] Run the newer installer. Repeat the whole section with the other installer type.
- [ ] Confirm the installer finishes with exit code `0`.
- [ ] Launch `windowcraft.exe` and navigate to the **`Time Machine`** module.
- [ ] **Expected result:** All prior sessions, tweak modification records, and snapshot tables remain intact without data loss or corruption.

---

## 12. Uninstallation

### Verify clean removal of application data
- [ ] Install WindowCraft via `setup.exe` or `MSI`.
- [ ] Apply at least one tweak and enable a feature (such as `reg-backup` and OpenSSH Server) to create data in `%LOCALAPPDATA%\WindowCraft`.
- [ ] Close WindowCraft.
- [ ] Uninstall WindowCraft via Windows Settings -> **`Apps`** -> **`Installed apps`** -> **`Uninstall`** (or `msiexec /x <ProductCode> /qn`).
- [ ] Verify file and task cleanup in elevated PowerShell:
  ```powershell
  Test-Path "$env:LOCALAPPDATA\WindowCraft"
  Get-ChildItem -Path "C:\Users" | ForEach-Object { Test-Path "$($_.FullName)\AppData\Local\WindowCraft" }
  schtasks /query /tn "AutoRegBackup" 2>$null
  Test-Path "$env:SystemRoot\SoftwareDistribution\Download.wcbak", "$env:SystemRoot\SoftwareDistribution\DataStore.wcbak", "$env:SystemRoot\System32\catroot2.wcbak"
  ```
- [ ] **Expected result:**
  - `%LOCALAPPDATA%\WindowCraft` is deleted across all user profiles.
  - The `AutoRegBackup` scheduled task is deleted.
  - Any `*.wcbak` folders left by Windows Update resets are deleted.
  - The OpenSSH firewall rule survives uninstallation on purpose.
  - Tweaks and packages modified on the live system remain in their modified state on purpose.
- [ ] Reinstall WindowCraft, launch it, start a long-running job, and attempt uninstallation while the process runs to confirm clean process handling.

---

## 13. Portable Build Purge (`--purge-data`)

### Verify `--purge-data` from elevated terminal
- [ ] Copy `WindowCraft-0.1.0-x64-portable.exe` to `C:\Test\windowcraft.exe`.
- [ ] Run tweaks to generate history in `%LOCALAPPDATA%\WindowCraft`.
- [ ] Open an elevated PowerShell prompt and run:
  ```powershell
  Start-Process -Wait C:\Test\windowcraft.exe -ArgumentList "--purge-data"
  ```
- [ ] **Expected result:**
  - `%LOCALAPPDATA%\WindowCraft` is deleted for all user profiles.
  - The `AutoRegBackup` scheduled task and `*.wcbak` folders are deleted.
- [ ] Recreate test data in `%LOCALAPPDATA%\WindowCraft`.
- [ ] Open an **unelevated** PowerShell prompt and run:
  ```powershell
  Start-Process -Wait C:\Test\windowcraft.exe -ArgumentList "--purge-data"
  ```
- [ ] **Expected result:** Only the active user's `%LOCALAPPDATA%\WindowCraft` folder is deleted; other user directories and system tasks remain untouched.

---

## Summary of Actions That Cannot Be Rolled Back

| Area | Action | Why It Cannot Be Rolled Back by Time Machine | Recovery Path in VM |
| :--- | :--- | :--- | :--- |
| **AppX Removal** | Removing / deprovisioning AppX packages (`appx_remove`) | Package payloads are deleted from the system and deprovisioned for all users. | Reinstall via **`Install`** in the `AppX Packages` tab (`winget` / Microsoft Store) or restore VM snapshot. |
| **Software & Apps** | Uninstalling Win32/Store apps (`winget uninstall`) | Application binaries and uninstaller state are removed externally by `winget`. | Reinstall via **`Install Selected`** in `Software & Apps` or restore VM snapshot. |
| **BitLocker** | **`BitLocker - Disable`** (`disable-bit-locker`) | Marked `irreversible: true`; decrypts the OS volume via WMI `Win32_EncryptableVolume.Decrypt`. | Re-enable BitLocker manually with `manage-bde -on C:` or restore VM snapshot. |
| **USB Write** | **`Erase and Write`** in ISO Studio | Wipes partition table, formats FAT32, and overwrites the target USB drive. | Reformat the USB drive manually. |

---

## What to Report Back

After completing the checklist in the VM, export the privacy-safe environment report:
```powershell
C:\Test\windowcraft-cli.exe --export-report C:\Test\environment-report.json
```
*(Verify `C:\Test\environment-report.json` contains OS build, CPU/GPU/memory summary, active power plan GUID, Windows Update mode, applied tweak count, and session history, and no username, computer name, serial numbers, or file paths.)*

Include the following in your test report:
- [ ] **Checklist status:** Which sections passed and which checkboxes failed or behaved unexpectedly.
- [ ] **Environment report:** Attach `C:\Test\environment-report.json`.
- [ ] **Job log:** Attach `%LOCALAPPDATA%\WindowCraft\windowcraft.log` (and `windowcraft.log.old` if present).
- [ ] **Command outputs:** Paste the exact output of any failed `reg query`, `sc qc`, `Get-AppxPackage`, `dism`, `manage-bde`, or `Get-Disk` verification command alongside the error message shown in the WindowCraft banner or CLI stderr.
- [ ] **OS details:** Windows 11 edition, build number (`winver`), guest language/locale, and hypervisor used (VirtualBox or VMware version).
