; Display branding can change; installed identity and user-owned paths cannot.
; These names match the existing 0.1.0 Windows packages. The pre-rename main
; binary name is kept only to recognise and retarget shortcuts that still point
; at it; the renamed product ships a single executable name.
!define PKMS_LEGACY_PRODUCTNAME "PKMS"
!define PKMS_LEGACY_MANUFACTURER "pkms"
!define PKMS_LEGACY_MAINBINARYNAME "pkms-desktop"

!macro SPANSEEK_RegisterPdfOpenWith
  ; OpenWith candidates only: never change .pdf's default or UserChoice.
  WriteRegStr SHCTX "Software\Classes\${BUNDLEID}.pdf" "" "${PRODUCTNAME} PDF document"
  WriteRegStr SHCTX "Software\Classes\${BUNDLEID}.pdf\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\${BUNDLEID}.pdf\shell\open" "" "Open with ${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\${BUNDLEID}.pdf\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  WriteRegStr SHCTX "Software\Classes\.pdf\OpenWithProgids" "${BUNDLEID}.pdf" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe" "FriendlyAppName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".pdf" ""
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro SPANSEEK_UnregisterPdfOpenWith
  ; A different installation may have taken ownership. Remove only our target.
  ReadRegStr $R7 SHCTX "Software\Classes\${BUNDLEID}.pdf\shell\open\command" ""
  ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    DeleteRegValue SHCTX "Software\Classes\.pdf\OpenWithProgids" "${BUNDLEID}.pdf"
    DeleteRegKey /ifempty SHCTX "Software\Classes\.pdf\OpenWithProgids"
    DeleteRegKey SHCTX "Software\Classes\${BUNDLEID}.pdf"
  ${EndIf}
  ReadRegStr $R7 SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" ""
  ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    DeleteRegKey SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe"
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro SPANSEEK_MigrateBrandShortcuts
  ${If} $NoShortcutMode != 1
    Push "$DESKTOP\${PKMS_LEGACY_PRODUCTNAME}.lnk"
    Push "$DESKTOP\${PRODUCTNAME}.lnk"
    Call spanseek.MigrateShortcut
    Push "$SMPROGRAMS\${PKMS_LEGACY_PRODUCTNAME}.lnk"
    Push "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Call spanseek.MigrateShortcut
    ${If} $AppStartMenuFolder != ""
      Push "$SMPROGRAMS\$AppStartMenuFolder\${PKMS_LEGACY_PRODUCTNAME}.lnk"
      Push "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
      Call spanseek.MigrateShortcut
    ${EndIf}
    !if "${INSTALLMODE}" == "currentUser"
      ; An existing taskbar pin is a separate shortcut copy. Preserve its AppID
      ; and notify Shell of the rename; never edit Taskband or pin a new item.
      Push "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\${PKMS_LEGACY_PRODUCTNAME}.lnk"
      Push "$APPDATA\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\${PRODUCTNAME}.lnk"
      Call spanseek.MigrateShortcut
    !endif
  ${EndIf}
!macroend

!macro SPANSEEK_CreateOwnedShortcut SHORTCUT_PATH
  Push "${SHORTCUT_PATH}"
  Call spanseek.CreateOwnedShortcut
!macroend

!macro SPANSEEK_BRAND_FUNCTIONS
Var SpanseekShortcutOld
Var SpanseekShortcutNew
Var SpanseekShortcutDestination

Function spanseek.MigrateShortcut
  Pop $SpanseekShortcutNew
  Pop $SpanseekShortcutOld
  IfFileExists "$SpanseekShortcutOld" 0 spanseek_shortcut_done
  !insertmacro IsShortcutTarget "$SpanseekShortcutOld" "$INSTDIR\${PKMS_LEGACY_MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 != 1
    ; Also finish an interrupted rename after the target was already updated.
    !insertmacro IsShortcutTarget "$SpanseekShortcutOld" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      Goto spanseek_shortcut_done
    ${EndIf}
  ${EndIf}

  !insertmacro SetShortcutTarget "$SpanseekShortcutOld" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$SpanseekShortcutOld"
  !insertmacro IsShortcutTarget "$SpanseekShortcutOld" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 != 1
    Goto spanseek_shortcut_failed
  ${EndIf}

  IfFileExists "$SpanseekShortcutNew" 0 spanseek_shortcut_rename
  !insertmacro IsShortcutTarget "$SpanseekShortcutNew" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 != 1
    ; A same-named shortcut belonging to another app must never be overwritten.
    Goto spanseek_shortcut_failed
  ${EndIf}
  ClearErrors
  Delete "$SpanseekShortcutOld"
  IfErrors spanseek_shortcut_failed
  System::Call 'shell32::SHChangeNotify(i 0x00000004, i 0x0005, w "$SpanseekShortcutOld", p 0)'
  Goto spanseek_shortcut_done

  spanseek_shortcut_rename:
    ClearErrors
    Rename "$SpanseekShortcutOld" "$SpanseekShortcutNew"
    IfErrors spanseek_shortcut_failed
    System::Call 'shell32::SHChangeNotify(i 0x00000001, i 0x0005, w "$SpanseekShortcutOld", w "$SpanseekShortcutNew")'
    Goto spanseek_shortcut_done

  spanseek_shortcut_failed:
    SetErrorLevel 1603
    Abort "A previous application shortcut could not be renamed to ${PRODUCTNAME}. Existing shortcuts were retained. Resolve the name or permission conflict and retry setup."
  spanseek_shortcut_done:
FunctionEnd

Function spanseek.CreateOwnedShortcut
  Pop $SpanseekShortcutDestination
  ; The legacy .lnk may be absent or user-renamed. The final destination must
  ; still be checked before creating anything under the new product name.
  IfFileExists "$SpanseekShortcutDestination" 0 spanseek_shortcut_create
  !insertmacro IsShortcutTarget "$SpanseekShortcutDestination" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 != 1
    !insertmacro IsShortcutTarget "$SpanseekShortcutDestination" "$INSTDIR\${PKMS_LEGACY_MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
    ${AndIf} $OldMainBinaryName != ""
      !insertmacro IsShortcutTarget "$SpanseekShortcutDestination" "$INSTDIR\$OldMainBinaryName"
      Pop $0
    ${EndIf}
  ${EndIf}
  ${If} $0 != 1
    Goto spanseek_shortcut_create_failed
  ${EndIf}
  ; Preserve arguments and other user settings on an owned existing shortcut.
  !insertmacro SetShortcutTarget "$SpanseekShortcutDestination" "$INSTDIR\${MAINBINARYNAME}.exe"
  Goto spanseek_shortcut_create_verify

  spanseek_shortcut_create:
    ClearErrors
    CreateShortcut "$SpanseekShortcutDestination" "$INSTDIR\${MAINBINARYNAME}.exe"
    IfErrors spanseek_shortcut_create_failed
  spanseek_shortcut_create_verify:
    !insertmacro SetLnkAppUserModelId "$SpanseekShortcutDestination"
    !insertmacro IsShortcutTarget "$SpanseekShortcutDestination" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      Goto spanseek_shortcut_create_failed
    ${EndIf}
    Return

  spanseek_shortcut_create_failed:
    SetErrorLevel 1603
    Abort "The ${PRODUCTNAME} shortcut could not be created or updated. Existing shortcuts were retained. Resolve the name or permission conflict and retry setup."
FunctionEnd
!macroend
