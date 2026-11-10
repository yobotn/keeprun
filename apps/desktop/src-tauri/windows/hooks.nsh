; Keeprun NSIS installer hooks. Installs the agent as a Windows service.
; Silent install for Intune / SCCM: Keeprun_x.y.z_x64-setup.exe /S

!macro NSIS_HOOK_PREINSTALL
  ; Stop a running agent so its files can be replaced on upgrade. Apps restart with it.
  nsExec::ExecToLog '"$SYSDIR\net.exe" stop KeeprunAgent'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Creates or updates the service, sets recovery actions, grants the installing user
  ; access to the API token, adds the CLI to PATH and starts the service.
  nsExec::ExecToLog '"$INSTDIR\keeprun-agent.exe" install --env prod'
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Keeprun was installed, but the agent service could not be set up (exit code $0). Open an Administrator terminal in $INSTDIR and run: keeprun-agent.exe install" /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Stops and removes the service and the PATH entry. Data in C:\ProgramData\Keeprun is kept.
  nsExec::ExecToLog '"$INSTDIR\keeprun-agent.exe" uninstall --env prod'
  Pop $0
!macroend
