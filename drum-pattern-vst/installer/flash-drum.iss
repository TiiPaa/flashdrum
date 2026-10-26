; Flash Drum VST3 — installer script (Inno Setup 6)
;
; Build: iscc installer\flash-drum.iss   (run from drum-pattern-vst/,
; after build.ps1 produced build\drum-pattern-vst.vst3\)
; or: .\build.ps1 -Installer
;
; Version must match drum-pattern-vst/Cargo.toml.

#define AppVersion "0.9.7"

[Setup]
AppName=Flash Drum
AppVersion={#AppVersion}
AppPublisher=Flash Drum
; VST3 system folder: C:\Program Files\Common Files\VST3
; ({commoncf} resolves to the 64-bit Common Files in x64 install mode)
DefaultDirName={commoncf}\VST3\drum-pattern-vst.vst3
DisableDirPage=yes
OutputDir=..\..\dist
OutputBaseFilename=FlashDrum-Setup-{#AppVersion}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
; [277] The GPL is shown for information: it asks for no acceptance to use
; the software (an InfoBefore page has Next, not "I accept").
InfoBeforeFile=..\..\LICENSE
; [277] Plain text, so the page reads cleanly (a .md showed its # and **).
InfoAfterFile=..\..\THIRD-PARTY.txt
Compression=lzma2
SolidCompression=yes
UninstallDisplayName=Flash Drum
; [277] The uninstaller lives OUTSIDE the plugin folder (default {app} = the
; .vst3 bundle itself): build.ps1 -Install swaps that whole folder and used
; to delete it. Setups built before [277] (2026-10-01) left unins000.* inside
; the bundle; an upgrade over one of those leaves that old pair behind,
; harmless.
UninstallFilesDir={autopf}\Flash Drum
WizardStyle=modern

[Files]
Source: "..\build\drum-pattern-vst.vst3\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion
