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
; Tauri includes this file before its own defines, variables and pages, so
; the key is spelled out and the functions use only registers and what NSIS
; itself defines. MUI2 calls the first function from .onGUIInit, after .onInit
; has chosen the registry view (SHCTX) and the install folder.

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

; An update over an older Cairn in the same folder goes from the Welcome page
; straight to Installing, as "Do not uninstall" on Tauri's "Already Installed"
; page would. That page preselects "Uninstall before installing", which runs
; the old version's uninstaller in a second window: it offers to delete the
; app data, unpins Cairn from the taskbar and makes its shortcuts again. This
; jumps only when the uninstall entry names the folder .onInit restored and
; this setup is newer than the entry's version: the same version, a downgrade,
; another folder (/D=) or an entry without a version get Tauri's page. Unlike
; Tauri's /UPDATE, a missing Start menu shortcut or WebView2 is installed.
; MUI2 gives this leave function to the first page, Welcome; /P skips Welcome
; and /S shows no pages, so neither changes. The jump counts Tauri's pages:
; Welcome, Already Installed, the folder, the Start menu (always skipped),
; then Installing; the test in src/lib.rs checks that order.
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE CairnUpdateInPlace

Function CairnUpdateInPlace
  Push $0
  Push $1
  ReadRegStr $0 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\Cairn" "UninstallString"
  StrCmp $0 '"$INSTDIR\uninstall.exe"' 0 done
  IfFileExists "$INSTDIR\uninstall.exe" 0 done
  ReadRegStr $0 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\Cairn" "DisplayVersion"
  StrCmp $0 "" done
  ClearErrors
  ; Tauri's VIProductVersion "x.y.z.0" is also this setup's file version.
  ${GetFileVersion} "$EXEPATH" $1
  IfErrors done
  ${VersionCompare} $1 $0 $1
  StrCmp $1 1 0 done
  Pop $1
  Pop $0
  ; WM_NOTIFY_OUTER_NEXT with 4: past Already Installed, the folder page and
  ; the Start menu page, to Installing. Abort ends this leave function, and
  ; NSIS then moves the 4 pages sent.
  SendMessage $HWNDPARENT 0x408 4 ""
  Abort
  done:
  Pop $1
  Pop $0
FunctionEnd
