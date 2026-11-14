; FreeType 安装器钩子（mygo 生成的 NSIS 安装器会包含本文件，放在 MUI2.nsh 和各语言之后）。
;
; 安装器按当前用户安装、不要管理员权限；只有注册系统输入法这一步用 runas 单独提权，
; 由 bin\FreeTypeServer.exe --register-tsf 把输入法 DLL 放进 Program Files 并注册。
;
; 安装器的语言跟随系统显示语言（简体中文、繁体中文、英文，见 locales\）；
; 下面自己的提示也按这三种语言各写一份。

!ifndef FREETYPE_VERSION
  !define FREETYPE_VERSION "0.0.1"
!endif
VIProductVersion "${FREETYPE_VERSION}.0"
VIFileVersion "${FREETYPE_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "FreeType"
VIAddVersionKey /LANG=1033 "ProductVersion" "${FREETYPE_VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${FREETYPE_VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName" "FreeType"
VIAddVersionKey /LANG=1033 "FileDescription" "FreeType Setup"
VIAddVersionKey /LANG=1033 "OriginalFilename" "FreeType-${FREETYPE_VERSION}-Setup.exe"

LangString FT_REGISTER_FAILED ${LANG_SIMPCHINESE} "输入法组件注册失败。请重新运行安装器，并在系统提示时允许管理员操作。"
LangString FT_REGISTER_FAILED ${LANG_TRADCHINESE} "輸入法元件註冊失敗。請重新執行安裝程式，並在系統提示時允許系統管理員操作。"
LangString FT_REGISTER_FAILED ${LANG_ENGLISH} "FreeType could not register its input method components. Run the installer again and allow the administrator request."

LangString FT_STOP_FAILED ${LANG_SIMPCHINESE} "无法结束安装目录中的 FreeType 进程。文件尚未替换；请关闭 FreeType 后重新运行安装程序。"
LangString FT_STOP_FAILED ${LANG_TRADCHINESE} "無法結束安裝目錄中的 FreeType 處理程序。檔案尚未取代；請關閉 FreeType 後重新執行安裝程式。"
LangString FT_STOP_FAILED ${LANG_ENGLISH} "FreeType could not stop its processes in the installation directory. No files were replaced. Close FreeType and run the installer again."

LangString FT_ENABLE_FAILED ${LANG_SIMPCHINESE} "无法将 FreeType 加入当前用户的输入法列表。安装尚未完成，请重新运行安装器；问题持续时请查看服务日志。"
LangString FT_ENABLE_FAILED ${LANG_TRADCHINESE} "無法將 FreeType 加入目前使用者的輸入法清單。安裝尚未完成，請重新執行安裝程式；問題持續時請查看服務日誌。"
LangString FT_ENABLE_FAILED ${LANG_ENGLISH} "FreeType could not enable the input method for this user. Installation is incomplete. Run the installer again; if it still fails, check the service log."

LangString FT_DISABLE_FAILED ${LANG_SIMPCHINESE} "无法将 FreeType 移出当前用户的输入法列表。卸载已停止，程序和用户数据保留。请重新运行安装器修复后再卸载。"
LangString FT_DISABLE_FAILED ${LANG_TRADCHINESE} "無法將 FreeType 移出目前使用者的輸入法清單。解除安裝已停止，程式和使用者資料保留。請重新執行安裝程式修復後再解除安裝。"
LangString FT_DISABLE_FAILED ${LANG_ENGLISH} "FreeType could not disable the input method for this user. Uninstall stopped; the program and user data are kept. Run the installer to repair it before uninstalling again."

LangString FT_UNREGISTER_FAILED ${LANG_SIMPCHINESE} "管理员操作已取消或输入法组件注销失败。卸载已停止，程序、启动项和用户数据保留。请重试卸载；若输入法不能使用，请重新运行安装器修复。"
LangString FT_UNREGISTER_FAILED ${LANG_TRADCHINESE} "系統管理員操作已取消或輸入法元件取消註冊失敗。解除安裝已停止，程式、啟動項目和使用者資料保留。請重試；若輸入法無法使用，請重新執行安裝程式修復。"
LangString FT_UNREGISTER_FAILED ${LANG_ENGLISH} "The administrator request was cancelled or input method removal failed. Uninstall stopped; the program, startup entry and user data are kept. Retry uninstalling. If the input method does not work, run the installer to repair it."

LangString FT_RESTORE_FAILED ${LANG_SIMPCHINESE} "输入法组件未确认注销，且无法恢复当前用户的输入法列表。卸载已停止，程序、启动项和用户数据保留。请重新运行安装器修复后再卸载。"
LangString FT_RESTORE_FAILED ${LANG_TRADCHINESE} "輸入法元件尚未確認取消註冊，且無法還原目前使用者的輸入法清單。解除安裝已停止，程式、啟動項目和使用者資料保留。請重新執行安裝程式修復後再解除安裝。"
LangString FT_RESTORE_FAILED ${LANG_ENGLISH} "Input method removal was not confirmed, and FreeType could not restore the user's input method entry. Uninstall stopped; the program, startup entry and user data are kept. Run the installer to repair it before uninstalling again."

; 安装和卸载共用按安装路径检查的停止步骤，不按进程名结束其他实例。
!macro FreeTypePrepareFunction Prefix
Function ${Prefix}FreeTypePrepare
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
FunctionEnd
!macroend

!insertmacro FreeTypePrepareFunction ""
!insertmacro FreeTypePrepareFunction "un."

Section "-FreeType preparation"
  Call FreeTypePrepare
SectionEnd

Function FreeTypeEnableForUser
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --enable'
  Pop $0
  StrCmp $0 "0" enabled
    MessageBox MB_ICONSTOP|MB_OK "$(FT_ENABLE_FAILED)" /SD IDOK
    SetErrorLevel 4
    Quit
  enabled:
FunctionEnd

; 完成页由 mygo 无参数启动程序；进程环境标记保证它和这里的启动使用相同选项。
; 只设置安装器自己的环境，不写入用户或系统环境变量。
Function FreeTypeLaunchAfterInstall
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_COMPLETED", t "1")'
  Exec '"$INSTDIR\FreeType.exe" --after-install'
FunctionEnd

!macro customInstall
  InitPluginsDir
  Delete "$PLUGINSDIR\freetype-register.ok"
  IfFileExists "$PLUGINSDIR\freetype-register.ok" register_failed 0
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" '--register-tsf "$INSTDIR\bin" --marker "$PLUGINSDIR\freetype-register.ok"' SW_HIDE
  IfFileExists "$PLUGINSDIR\freetype-register.ok" registered register_failed
  register_failed:
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(FT_REGISTER_FAILED)" /SD IDOK
    ; Keep pending.json for a retry; do not launch an incompletely updated app.
    SetErrorLevel 3
    Quit
  registered:
  Delete "$PLUGINSDIR\freetype-register.ok"
  ; 加进当前用户的输入法列表，开机时放托盘图标并启动输入法服务
  Call FreeTypeEnableForUser
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType" '"$INSTDIR\FreeType.exe" --background'
  ; The verified update is complete only after all input-method components register.
  Delete "$APPDATA\FreeType\updates\pending.json"
  Call FreeTypeLaunchAfterInstall
!macroend

!macro customUnInstall
  Call un.FreeTypePrepare
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --disable'
  Pop $0
  StrCmp $0 "0" disabled
    MessageBox MB_ICONSTOP|MB_OK "$(FT_DISABLE_FAILED)" /SD IDOK
    SetErrorLevel 5
    Quit
  disabled:
  ; ExecShellWait has no exit-code output. Only this invocation's marker permits cleanup.
  Delete "$PLUGINSDIR\freetype-unregister.ok"
  IfFileExists "$PLUGINSDIR\freetype-unregister.ok" unregister_failed 0
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" '--unregister-tsf --marker "$PLUGINSDIR\freetype-unregister.ok"' SW_HIDE
  IfFileExists "$PLUGINSDIR\freetype-unregister.ok" unregistered unregister_failed
  unregister_failed:
    nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --enable'
    Pop $0
    StrCmp $0 "0" user_reenabled
      MessageBox MB_ICONSTOP|MB_OK "$(FT_RESTORE_FAILED)" /SD IDOK
      SetErrorLevel 7
      Quit
    user_reenabled:
    MessageBox MB_ICONSTOP|MB_OK "$(FT_UNREGISTER_FAILED)" /SD IDOK
    SetErrorLevel 6
    Quit
  unregistered:
  Delete "$PLUGINSDIR\freetype-unregister.ok"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType"
!macroend
