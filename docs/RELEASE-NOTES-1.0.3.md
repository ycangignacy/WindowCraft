# WindowCraft 1.0.3

This release fixes bugs. Nothing that changes the system is different from 1.0.2.

## Control Panel buttons

On the System page, the buttons that open a Control Panel applet failed with a notice such as "ShellExecuteW failed for main.cpl" and error 0x0000001F. Ten buttons were affected: Mouse Properties, Network Connections, Power Panel, Programs and Features, Region, Security and Maintenance, Sound Settings, System Properties, Time and Date, and Windows Defender Firewall. WindowCraft asked Windows to `open` the `.cpl` file, and the `cplfile` type has no `open` verb; its default verb is `cplopen`. ShellExecute then returned code 31, which the notice showed with the unrelated system message for ERROR_GEN_FAILURE ("A device attached to the system is not functioning").

Applets now open through `control.exe`, which is how Windows itself starts them. Every other target, such as Computer Management or Windows Restore, opens with its default verb. When a launch does fail, the notice shows the real Windows message. Declining the UAC prompt for Windows Restore no longer shows an error.

## Uninstall closes an open WindowCraft

Uninstalling while WindowCraft was open removed its files and data, and the window kept running until it was closed (KB-9). The uninstaller now closes it first. A window in the user's session cannot be asked to close from the Windows Installer service, which runs as SYSTEM in another session, so before it removes files the uninstaller runs `windowcraft.exe --close-running`. That ends every process started from the program folder, `windowcraft-cli.exe` included, and waits up to 10 seconds for each to exit. A copy of the portable build somewhere else keeps running.

An upgrade from 1.0.3 to a later version removes 1.0.3 the same way, so it closes the app too. An upgrade from 1.0.2 to 1.0.3 does not: 1.0.2 has no such step. A job that is still running when the app is closed stops as if the process had crashed.

## Translations

The configuration drift banner and the drift details on the Time Machine page were in English in every language. They are translated into the other seven languages now.

## Tested

The unit and integration tests pass (432, with clippy clean at `-D warnings`). On the development PC, Windows 11 Home 26H2, build 26300, Polish, the Mouse Properties applet opened through the new code path, and a test checks that every applet the System page opens exists in System32. The other nine applets go through the same path but were not clicked one by one.

In VirtualBox on the same Windows build, from a clean snapshot:

| Check | Result |
| :--- | :--- |
| `verify-install.ps1` on the MSI, the MSI with `WCLANGUAGE=pl` and `setup.exe` | 57 of 57 passed, including a window opened before the MSI and the `setup.exe` uninstall and closed by it |
| MSI uninstall while the app is open | Exit 0, the window closed, data and program folder removed. The log shows 1 process closed 130 ms before the data was purged |
| MSI upgrade from 1.0.2 with a recorded session | Exit 0, history byte for byte the same, one uninstall entry, 1.0.3 reads the 1.0.2 history |
| `setup.exe` of 1.0.2-rc1, then of 1.0.2 (two entries, KB-10), then of 1.0.3 | Exit 0 each time; after 1.0.3 one entry is left, "WindowCraft 1.0.3", with one MSI product and the history byte for byte the same |
| Uninstall of that entry with the window open | Exit 0, the window closed, no entry, program folder or data left |
| Portable `--purge-data` | Data folder removed |

KB-10 is gone with this release: `setup.exe` of 1.0.3 replaces both entries that 1.0.2-rc1 and 1.0.2 left. Not tested: an uninstall from Settings with the full MSI interface, where Windows may first show its own "files in use" dialog.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

Mica shows behind the sidebar only on the CPU path.

The builds are not code-signed yet, so SmartScreen warns on first run.

ARM64: the files build and have the right platform, but no ARM64 binary has run on ARM hardware, and the GPU path is untested there.

Everything else in the limitations of [1.0.1](https://github.com/ycangignacy/WindowCraft/releases/tag/v1.0.1) still applies: Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption; an uninstall deletes Time Machine history for every user; the portable build has no uninstaller; and an ISO build installs `oscdimg` through WinGet when it is missing.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.0.3-x64-setup.exe`, `WindowCraft-1.0.3-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.0.3-x64.msi`, `WindowCraft-1.0.3-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.0.3-x64-portable.exe`, `WindowCraft-1.0.3-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |
