; Per-user installer: no administrator rights, installs to %LOCALAPPDATA%\Programs\Snowhush.
; Built by build.sh, which passes VERSION, EXE, SIZE (in KB) and OUTFILE.
Unicode true
ManifestDPIAware true
!include "MUI2.nsh"
!include "WinMessages.nsh"

!define UNINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snowhush"

Name "Snowhush"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Snowhush"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText " "

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Snowhush"
VIAddVersionKey "FileDescription" "Snowhush Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Mykhailo Bezruchko"
VIAddVersionKey "LegalCopyright" "© 2026 Mykhailo Bezruchko"

!define MUI_ICON "assets\Snowhush.ico"
!define MUI_UNICON "assets\Snowhush.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\Snowhush.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Snowhush"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; Asks a running Snowhush to quit, so its files can be replaced or removed.
!macro CloseSnowhush
  FindWindow $0 "SnowhushController"
  StrCmp $0 0 +3
    SendMessage $0 ${WM_CLOSE} 0 0
    Sleep 800
!macroend

Section
  !insertmacro CloseSnowhush
  SetOutPath "$INSTDIR"
  File "/oname=Snowhush.exe" "${EXE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Snowhush.lnk" "$INSTDIR\Snowhush.exe"
  WriteRegStr HKCU "${UNINSTALL}" "DisplayName" "Snowhush"
  WriteRegStr HKCU "${UNINSTALL}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL}" "Publisher" "Mykhailo Bezruchko"
  WriteRegStr HKCU "${UNINSTALL}" "DisplayIcon" "$INSTDIR\Snowhush.exe"
  WriteRegStr HKCU "${UNINSTALL}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL}" "URLInfoAbout" "https://snowhush.pages.dev"
  WriteRegStr HKCU "${UNINSTALL}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTALL}" "EstimatedSize" ${SIZE}
SectionEnd

Section "Uninstall"
  !insertmacro CloseSnowhush
  Delete "$INSTDIR\Snowhush.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Snowhush.lnk"
  DeleteRegKey HKCU "${UNINSTALL}"
  DeleteRegKey HKCU "Software\Snowhush"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Snowhush"
  RMDir /r "$LOCALAPPDATA\Snowhush"
SectionEnd
