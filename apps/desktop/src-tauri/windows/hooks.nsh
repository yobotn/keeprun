; Keeprun NSIS installer hooks. Installs the agent as a Windows service.
; Silent install for Intune / SCCM: Keeprun_x.y.z_x64-setup.exe /S
; Optional: /PORT=7272 sets the agent API port (default 7171).
; Optional: /NOUPDATES turns off update checks in the desktop app (policy value
; HKLM\SOFTWARE\Policies\Keeprun DisableUpdates, which Group Policy can also set).

!include "FileFunc.nsh"

!macro KEEPRUN_FOLDER_MENU KEY
  WriteRegStr HKLM "Software\Classes\${KEY}" "" "Open with Keeprun"
  WriteRegStr HKLM "Software\Classes\${KEY}" "Icon" '"$INSTDIR\keeprun-desktop.exe",0'
  WriteRegStr HKLM "Software\Classes\${KEY}\command" "" '"$INSTDIR\keeprun-desktop.exe" "%V"'
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Stop a running agent so its files can be replaced on upgrade. Apps restart with it.
  nsExec::ExecToLog '"$SYSDIR\net.exe" stop KeeprunAgent'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Creates or updates the service, sets recovery actions, grants the installing user
  ; access to the API token, adds the CLI to PATH and starts the service.
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/PORT=" $R1
  ${If} ${Errors}
    StrCpy $R1 ""
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/NOUPDATES" $R2
  ${IfNot} ${Errors}
    SetRegView 64
    WriteRegDWORD HKLM "SOFTWARE\Policies\Keeprun" "DisableUpdates" 1
  ${EndIf}
  ; Explorer "Open with Keeprun" on folders and on the empty space inside a folder.
  ; On Windows 11 it is under "Show more options" (Shift+F10).
  SetRegView 64
  !insertmacro KEEPRUN_FOLDER_MENU "Directory\shell\Keeprun"
  !insertmacro KEEPRUN_FOLDER_MENU "Directory\Background\shell\Keeprun"
  nsExec::ExecToLog '"$INSTDIR\keeprun-agent.exe" install --env prod --port "$R1"'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Keeprun was installed, but the agent service could not be set up (exit code $0). Open an Administrator terminal in $INSTDIR and run: keeprun-agent.exe install" /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Stops and removes the service and the PATH entry. Data in C:\ProgramData\Keeprun is kept.
  nsExec::ExecToLog '"$INSTDIR\keeprun-agent.exe" uninstall --env prod'
  Pop $0
  SetRegView 64
  DeleteRegKey HKLM "Software\Classes\Directory\shell\Keeprun"
  DeleteRegKey HKLM "Software\Classes\Directory\Background\shell\Keeprun"
!macroend
