; DBForge's Windows installer, built by scripts/make-windows.sh with makensis:
;
;   makensis -DVERSION=0.4.0 -DVERSIONWITHBUILD=0.4.0.0 -DSOURCE=<DBForge.exe> -DOUTFILE=<setup.exe> \
;            -DICON=<icon.ico> installer.nsi
;
; It installs for the current user, without an administrator, exactly where and as the Tauri app's NSIS
; installer did, so a new version replaces an installed one in place and every installed copy can update
; itself by running it:
;
;   - %LOCALAPPDATA%\DBForge\DBForge.exe and uninstall.exe, the folder remembered (and read back) in
;     HKCU\Software\dbforge\DBForge;
;   - the uninstall entry HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\DBForge with the same
;     values (DisplayName, DisplayVersion, Publisher "dbforge", MainBinaryName, UninstallString…);
;   - Start menu (and, when chosen, desktop) shortcuts named DBForge.lnk, carrying the AppUserModelID
;     dev.dbforge.app, so pins and taskbar grouping keep working.
;
; The updater (Tauri's in 0.3.x, the app's own since) runs it with /P /UPDATE /R /ARGS …:
;   /P       passive: a progress window and no questions;
;   /UPDATE  an update: shortcuts left as they are;
;   /R       start DBForge once installed, with the arguments after /ARGS;
;   /NS      (by hand) no shortcuts.
; Adapted from Tauri's NSIS template (MIT or Apache-2.0), without its WebView2 and plugin parts.

Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
SetCompressor /SOLID lzma

!ifndef VERSION
  !error "Pass -DVERSION=<version>"
!endif
!ifndef VERSIONWITHBUILD
  !error "Pass -DVERSIONWITHBUILD=<major.minor.patch.build>, numbers only"
!endif
!ifndef SOURCE
  !error "Pass -DSOURCE=<the built DBForge.exe>"
!endif
!ifndef OUTFILE
  !error "Pass -DOUTFILE=<the setup to write>"
!endif
!ifndef ICON
  !error "Pass -DICON=<icon.ico>"
!endif

!include MUI2.nsh
!include FileFunc.nsh
!include LogicLib.nsh
!include x64.nsh
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "Win\RestartManager.nsh"

!define PRODUCTNAME "DBForge"
!define MAINBINARYNAME "DBForge"
!define BUNDLEID "dev.dbforge.app"
; Tauri's default publisher: the identifier's second part.
!define MANUFACTURER "dbforge"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY "${MANUKEY}\${PRODUCTNAME}"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
; A placeholder: .onInit sets the folder Tauri's installer used, or the one already installed to.
!define PLACEHOLDER_INSTALL_DIR "placeholder\${PRODUCTNAME}"
InstallDir "${PLACEHOLDER_INSTALL_DIR}"
RequestExecutionLevel user

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" ""
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; Pages. Passive and update runs skip every page but the progress.
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassiveOrUpdate
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
; The finish page's first box creates a desktop shortcut; its second starts DBForge.
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Create desktop shortcut"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateDesktopShortcut
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

LangString appRunning ${LANG_ENGLISH} "DBForge is running! Please close it first and try again."
LangString appRunningOkKill ${LANG_ENGLISH} "DBForge is running!$\nClick OK to close it"
LangString failedToKillApp ${LANG_ENGLISH} "Failed to close DBForge. Please close it first and try again"

; The current user's shell folders and registry, in the 64-bit view, as Tauri's installer used.
!macro SetContext
  SetShellVarContext current
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
!macroend

!define /ifndef ERROR_MORE_DATA 234

; Closes a running DBForge (an agent's `--mcp-stdio` included) through the Restart Manager, asking first
; unless passive or silent, as Tauri's installer did.
!macro CheckIfAppIsRunning
  !define UniqueID ${__LINE__}
  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 != ""
    !insertmacro RestartManager_RegisterFile $R0 "$INSTDIR\${MAINBINARYNAME}.exe"
    ${If} $0 = 0
      System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
      ${If} $0 = ${ERROR_MORE_DATA}
        IfSilent kill_${UniqueID} 0
        ${IfThen} $PassiveMode != 1 ${|} MessageBox MB_OKCANCEL "$(appRunningOkKill)" IDOK kill_${UniqueID} IDCANCEL cancel_${UniqueID} ${|}
        kill_${UniqueID}:
          System::Call 'RSTRTMGR::RmShutdown(p R0, i ${RmForceShutdown}, p 0) i .r0'
          ${If} $0 = 0
            Goto done_${UniqueID}
          ${EndIf}
          !insertmacro RestartManager_EndSession $R0
          Abort "$(failedToKillApp)"
        cancel_${UniqueID}:
          !insertmacro RestartManager_EndSession $R0
          Abort "$(appRunning)"
        done_${UniqueID}:
      ${EndIf}
    ${EndIf}
    !insertmacro RestartManager_EndSession $R0
  ${EndIf}
  !undef UniqueID
!macroend

; Gives a shortcut DBForge's AppUserModelID, so the running app and its pins group together.
!macro SetLnkAppUserModelId shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
      ${If} $2 P<> 0
        System::Call 'Oleaut32::SysAllocString(w "${BUNDLEID}") i.r3'
        System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID})p.r4'
        System::Call '*${SYSSTRUCT_PROPVARIANT}(${VT_BSTR},,&i4 $3)p.r5'
        ${IPropertyStore::SetValue} $2 '($4,$5)'
        System::Call 'Oleaut32::SysFreeString($3)'
        System::Free $4
        System::Free $5
        ${IPropertyStore::Commit} $2 ""
        ${IUnknown::Release} $2 ""
        ${IPersistFile::Save} $1 '("${shortcut}",1)'
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}

  !insertmacro SetContext

  ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
    StrCpy $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}"
    ; Where DBForge already is, wherever the user put it.
    ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
    ${If} $4 != ""
      StrCpy $INSTDIR $4
    ${EndIf}
  ${EndIf}
FunctionEnd

Section Install
  SetOutPath $INSTDIR
  !insertmacro CheckIfAppIsRunning

  File "/oname=${MAINBINARYNAME}.exe" "${SOURCE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; The folder, for the next install and update.
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR

  ; Add or remove programs.
  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"

  ; An update keeps the shortcuts as they are: they already open this folder's DBForge.exe.
  ${If} $UpdateMode <> 1
  ${AndIf} $NoShortcutMode <> 1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ; The finish page offers the desktop one; a passive or silent install skips that page.
    ${If} $PassiveMode = 1
    ${OrIf} ${Silent}
      Call CreateDesktopShortcut
    ${EndIf}
  ${EndIf}

  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ; Passive and silent runs restart DBForge when asked (/R); the finish page offers it otherwise.
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $R0
      Exec '"$INSTDIR\${MAINBINARYNAME}.exe" $R0'
    ${EndIf}
  ${EndIf}
FunctionEnd

Function CreateDesktopShortcut
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
FunctionEnd

Function RunMainBinary
  Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
FunctionEnd

Function SkipIfPassiveOrUpdate
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
  ${IfThen} $UpdateMode = 1 ${|} Abort ${|}
FunctionEnd

Function un.onInit
  !insertmacro SetContext
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
FunctionEnd

Function un.SkipIfPassive
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
FunctionEnd

; Uninstalling keeps the user's connections and settings (in %APPDATA%\dev.dbforge.app).
Section Uninstall
  !insertmacro CheckIfAppIsRunning

  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
  ${EndIf}
  DeleteRegKey SHCTX "${UNINSTKEY}"

  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd
