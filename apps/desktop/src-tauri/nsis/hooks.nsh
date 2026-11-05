; spanseek keeps uncommitted editor/IME state in its running WebView. Installers
; must require a normal application exit, never force it through Restart Manager.
; Tauri 2.12.1 includes installerHooks after utils.nsh and before its pages.

!ifndef PKMS_INSTALLER_HOOKS_INCLUDED
!define PKMS_INSTALLER_HOOKS_INCLUDED

!include "${__FILEDIR__}\brand.nsh"

!macro PKMS_RequireClosed executablePath productName
  !define PKMS_GUARD_ID ${__LINE__}
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $R0

  ; A fresh install does not contain the legacy helper names, and the first
  ; install has no main executable yet. A missing file cannot be running, so
  ; skip the Restart Manager probe instead of treating it as a failed probe.
  ${IfNot} ${FileExists} "${executablePath}"
    Goto pkms_guard_done_${PKMS_GUARD_ID}
  ${EndIf}
  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 == ""
    Goto pkms_probe_failed_${PKMS_GUARD_ID}
  ${EndIf}
  !insertmacro RestartManager_RegisterFile $R0 "${executablePath}"
  ${If} $0 <> 0
    !insertmacro RestartManager_EndSession $R0
    Goto pkms_probe_failed_${PKMS_GUARD_ID}
  ${EndIf}
  StrCpy $1 0
  StrCpy $2 0
  StrCpy $3 0
  System::Call 'RSTRTMGR::RmGetList(i R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
  !insertmacro RestartManager_EndSession $R0
  ${If} $0 = ${ERROR_MORE_DATA}
    StrCpy $4 "${productName} 仍在运行：请保存工作并从应用内正常退出后重试。本安装程序不会强制关闭它。$\r$\n$\r$\n${productName} is still running. Save your work and exit from the application, then retry. Setup will not force it to close."
    SetErrorLevel 1618
    Goto pkms_refuse_${PKMS_GUARD_ID}
  ${ElseIf} $0 <> 0
    Goto pkms_probe_failed_${PKMS_GUARD_ID}
  ${EndIf}
  Goto pkms_guard_done_${PKMS_GUARD_ID}

  pkms_probe_failed_${PKMS_GUARD_ID}:
    StrCpy $4 "无法确认 ${productName} 是否已退出，未改动任何程序文件：请关闭应用后重试。$\r$\n$\r$\nSetup could not verify that ${productName} has exited. No program files were changed. Close the application and retry."
    SetErrorLevel 1603
  pkms_refuse_${PKMS_GUARD_ID}:
    DetailPrint $4
    IfSilent pkms_quit_${PKMS_GUARD_ID}
    ; /P is also noninteractive. Do not leave a hidden modal prompt hanging.
    ClearErrors
    ${GetOptions} $CMDLINE "/P" $3
    IfErrors 0 pkms_quit_${PKMS_GUARD_ID}
    MessageBox MB_OK|MB_ICONEXCLAMATION $4
  pkms_quit_${PKMS_GUARD_ID}:
    Quit
  pkms_guard_done_${PKMS_GUARD_ID}:
    Pop $R0
    Pop $4
    Pop $3
    Pop $2
    Pop $1
    Pop $0
  !undef PKMS_GUARD_ID
!macroend

; Replacing the original macro removes its forced shutdown even if the app
; starts between our PREINSTALL check and Tauri's own process check.
!macroundef CheckIfAppIsRunning
!macro CheckIfAppIsRunning executablePath productName
  !insertmacro PKMS_RequireClosed "${executablePath}" "${productName}"
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro PKMS_RequireInstallClosed
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro PKMS_RequireInstallClosed
!macroend

; Setup installs new files but never deletes a renamed one. Drop the pre-rename
; helper names so an updated directory shows one complete product name.
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\pkms-worker.exe"
  Delete "$INSTDIR\pkms-inference.exe"
  ; The renamed product ships one executable name. Shortcuts that still point
  ; at the pre-rename launcher were retargeted before this cleanup ran.
  Delete "$INSTDIR\${PKMS_LEGACY_MAINBINARYNAME}.exe"
!macroend

!macro PKMS_RequireInstallClosed
  !insertmacro PKMS_RequireClosed "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  !insertmacro PKMS_RequireClosed "$INSTDIR\${PKMS_LEGACY_MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  !insertmacro PKMS_RequireClosed "$INSTDIR\spanseek-worker.exe" "${PRODUCTNAME}"
  !insertmacro PKMS_RequireClosed "$INSTDIR\spanseek-inference.exe" "${PRODUCTNAME}"
  !insertmacro PKMS_RequireClosed "$INSTDIR\pkms-worker.exe" "${PRODUCTNAME}"
  !insertmacro PKMS_RequireClosed "$INSTDIR\pkms-inference.exe" "${PRODUCTNAME}"
!macroend

!endif
