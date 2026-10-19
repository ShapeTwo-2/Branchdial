; Branchdial NSIS installer hooks (bundle.windows.nsis.installerHooks).
; Adds "Open Branchdial here" to Explorer's right-click menu: on a folder,
; and on the empty space inside a folder. SHCTX follows the install mode
; (per-user -> HKCU, per-machine -> HKLM).

!define RB_BG_KEY "Software\Classes\Directory\Background\shell\Branchdial"
!define RB_DIR_KEY "Software\Classes\Directory\shell\Branchdial"

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "${RB_BG_KEY}" "" "Open Branchdial here"
  WriteRegStr SHCTX "${RB_BG_KEY}" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${RB_BG_KEY}\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%V$\""

  WriteRegStr SHCTX "${RB_DIR_KEY}" "" "Open Branchdial here"
  WriteRegStr SHCTX "${RB_DIR_KEY}" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${RB_DIR_KEY}\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey SHCTX "${RB_BG_KEY}"
  DeleteRegKey SHCTX "${RB_DIR_KEY}"
!macroend
