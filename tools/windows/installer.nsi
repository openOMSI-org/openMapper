Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "x64.nsh"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\openMapper"

Name "openMapper"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\openMapper"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "openMapper ${VERSION}"

!define MUI_ICON "assets\icons\app\openmapper.ico"
!define MUI_UNICON "assets\icons\app\openmapper.ico"
!define MUI_BGCOLOR 14161A
!define MUI_TEXTCOLOR EDEDED
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "assets\icons\app\installer-header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "assets\icons\app\installer-header.bmp"
!define MUI_WELCOMEFINISHPAGE_BITMAP "assets\icons\app\installer-sidebar.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "assets\icons\app\installer-sidebar.bmp"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\openmapper.exe"

; (a program built for the other processor only says "This app can't run on your PC" when
; started: the installer says which package this computer needs instead, #102)
Function .onInit
!if "${ARCH}" == "arm64"
  ${IfNot} ${IsNativeARM64}
    MessageBox MB_ICONSTOP "This is openMapper for ARM64 processors. This computer has an Intel or AMD processor: download openMapper-${VERSION}-windows-x64-setup.exe instead.$\r$\n$\r$\nЭто openMapper для процессоров ARM64. В этом компьютере процессор Intel или AMD: скачайте openMapper-${VERSION}-windows-x64-setup.exe."
    Abort
  ${EndIf}
!else
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "openMapper needs a 64-bit Windows 10 or 11; this Windows is 32-bit.$\r$\n$\r$\nopenMapper нужна 64-битная Windows 10 или 11, а эта Windows 32-битная."
    Abort
  ${EndIf}
!endif
FunctionEnd

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishShow
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "Italian"
!insertmacro MUI_LANGUAGE "Dutch"
!insertmacro MUI_LANGUAGE "Polish"
!insertmacro MUI_LANGUAGE "Czech"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "Ukrainian"
!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "Korean"

Section
  SetOutPath "$INSTDIR"
  File "${DIST}\openmapper.exe"
  SetOutPath "$INSTDIR\docs"
  File "${DIST}\docs\TUTORIAL.md"
  SetOutPath "$INSTDIR"
  File "${DIST}\README.md"
  File /nonfatal "${DIST}\LICENSE"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\openMapper.lnk" "$INSTDIR\openmapper.exe"
  CreateShortcut "$DESKTOP\openMapper.lnk" "$INSTDIR\openmapper.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "openMapper"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\openmapper.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "openOMSI"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/openOMSI-org/openMapper"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Function FinishShow
  System::Call 'uxtheme::SetWindowTheme(p $mui.FinishPage.Run, w " ", w " ")'
  SetCtlColors $mui.FinishPage.Run EDEDED 14161A
FunctionEnd

; Only what the installer put there goes: maps and settings live in ~/.openomsi, never in the folder.
Section "Uninstall"
  Delete "$INSTDIR\openmapper.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\docs\TUTORIAL.md"
  RMDir "$INSTDIR\docs"
  Delete "$INSTDIR\*.old-update"
  Delete "$INSTDIR\*.bad-update"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\openMapper.lnk"
  Delete "$DESKTOP\openMapper.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
