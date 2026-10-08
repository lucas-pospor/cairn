; Cairn's additions to Tauri's NSIS installer (bundle.windows.nsis.installerHooks
; in tauri.conf.json).
;
; An install whose folder was deleted without its uninstaller leaves its
; uninstall entry behind. Tauri's installer then shows "Already Installed",
; runs the missing uninstall.exe when "Uninstall before installing" is
; chosen (the default for an upgrade), says "Unable to uninstall!" and goes
; back to the same page, again and again. Before the first page, this drops
; an entry whose uninstaller is gone, so the installer goes on as a fresh
; install. A silent install (/S) shows no pages and never had the problem.
;
; Tauri includes this file before its own defines and pages, so the key is
; spelled out, and MUI2 calls the function from .onGUIInit, after .onInit
; has chosen the registry view (SHCTX).

!define MUI_CUSTOMFUNCTION_GUIINIT CairnDropStaleUninstallEntry

Function CairnDropStaleUninstallEntry
  Push $0
  Push $1
  Push $2
  ReadRegStr $0 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\Cairn" "UninstallString"
  ; The value is the uninstaller's path in quotes; leave anything else alone.
  StrCpy $1 $0 1
  StrCmp $1 '"' 0 done
  StrCpy $0 $0 "" 1
  StrLen $2 $0
  IntOp $2 $2 - 1
  IntCmp $2 0 done done
  StrCpy $1 $0 1 $2
  StrCmp $1 '"' 0 done
  StrCpy $0 $0 $2
  IfFileExists $0 done
  DeleteRegKey SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\Cairn"
  done:
  Pop $2
  Pop $1
  Pop $0
FunctionEnd
