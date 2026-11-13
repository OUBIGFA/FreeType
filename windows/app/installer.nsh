; FreeType 安装器钩子（mygo 生成的 NSIS 安装器会包含本文件，放在 MUI2.nsh 和各语言之后）。
;
; 安装器按当前用户安装、不要管理员权限；只有注册系统输入法这一步用 runas 单独提权，
; 由 bin\FreeTypeServer.exe --register-tsf 把输入法 DLL 放进 Program Files 并注册。
;
; 安装器的语言跟随系统显示语言（简体中文、繁体中文、英文，见 locales\）；
; 下面自己的提示也按这三种语言各写一份。

LangString FT_REGISTER_FAILED ${LANG_SIMPCHINESE} "输入法组件注册失败。请重新运行安装器，并在系统提示时允许管理员操作。"
LangString FT_REGISTER_FAILED ${LANG_TRADCHINESE} "輸入法元件註冊失敗。請重新執行安裝程式，並在系統提示時允許系統管理員操作。"
LangString FT_REGISTER_FAILED ${LANG_ENGLISH} "FreeType could not register its input method components. Run the installer again and allow the administrator request."

LangString FT_STOP_FAILED ${LANG_SIMPCHINESE} "无法结束安装目录中的 FreeType 进程。文件尚未替换；请关闭 FreeType 后重新运行安装程序。"
LangString FT_STOP_FAILED ${LANG_TRADCHINESE} "無法結束安裝目錄中的 FreeType 處理程序。檔案尚未取代；請關閉 FreeType 後重新執行安裝程式。"
LangString FT_STOP_FAILED ${LANG_ENGLISH} "FreeType could not stop its processes in the installation directory. No files were replaced. Close FreeType and run the installer again."

Section "-FreeType preparation"
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=prepare-install.ps1 "${__FILEDIR__}\prepare-install.ps1"
  StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$0" +2 0
    StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_DIR", t "$INSTDIR")'
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_SCRIPT", t "$PLUGINSDIR\prepare-install.ps1")'
  nsExec::ExecToLog /TIMEOUT=30000 '"$0" -NoProfile -NonInteractive -Command "& ([scriptblock]::Create([IO.File]::ReadAllText($$env:FREETYPE_INSTALL_SCRIPT)))"'
  Pop $0
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_DIR", t "")'
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_SCRIPT", t "")'
  StrCmp $0 "0" prepared
    MessageBox MB_ICONSTOP|MB_OK "$(FT_STOP_FAILED)" /SD IDOK
    SetErrorLevel 2
    Quit
  prepared:
SectionEnd

!macro customInstall
  Delete "$TEMP\freetype-register.ok"
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" '--register-tsf "$INSTDIR\bin" --marker "$TEMP\freetype-register.ok"' SW_HIDE
  IfFileExists "$TEMP\freetype-register.ok" registered 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(FT_REGISTER_FAILED)" /SD IDOK
    ; Keep pending.json for a retry; do not launch an incompletely updated app.
    SetErrorLevel 3
    Quit
  registered:
  Delete "$TEMP\freetype-register.ok"
  ; 加进当前用户的输入法列表，开机时放托盘图标并启动输入法服务
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --enable'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType" '"$INSTDIR\FreeType.exe" --background'
  ; The verified update is complete only after all input-method components register.
  Delete "$APPDATA\FreeType\updates\pending.json"
  Exec '"$INSTDIR\FreeType.exe" --background'
!macroend

!macro customUnInstall
  nsExec::Exec 'taskkill /F /IM FreeType.exe'
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --stop'
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --disable'
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" "--unregister-tsf" SW_HIDE
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType"
!macroend
