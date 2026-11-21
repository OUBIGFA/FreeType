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

LangString FT_REGISTER_FAILED ${LANG_SIMPCHINESE} "输入法注册失败；请重新运行安装器并允许管理员操作"
LangString FT_REGISTER_FAILED ${LANG_TRADCHINESE} "輸入法註冊失敗；請重新執行安裝程式並允許系統管理員操作"
LangString FT_REGISTER_FAILED ${LANG_ENGLISH} "Input method registration failed； run setup again and allow administrator access"

LangString FT_STOP_FAILED ${LANG_SIMPCHINESE} "无法关闭安装目录中的 FreeType；文件尚未替换；请关闭 FreeType 后重试安装"
LangString FT_STOP_FAILED ${LANG_TRADCHINESE} "無法關閉安裝目錄中的 FreeType；檔案尚未取代；請關閉 FreeType 後重試安裝"
LangString FT_STOP_FAILED ${LANG_ENGLISH} "Cannot close FreeType in the installation folder； files were not replaced； close FreeType and run setup again"

LangString FT_START_FAILED ${LANG_SIMPCHINESE} "输入法服务未启动或词库未加载；更新未完成，待更新记录已保留；请查看服务日志后重试安装"
LangString FT_START_FAILED ${LANG_TRADCHINESE} "輸入法服務未啟動或詞庫未載入；更新未完成，待更新記錄已保留；請查看服務日誌後重試安裝"
LangString FT_START_FAILED ${LANG_ENGLISH} "The input service did not start or load the dictionary； the update is incomplete and its pending record is kept； check the service log and run setup again"

LangString FT_ENABLE_FAILED ${LANG_SIMPCHINESE} "无法加入当前用户的输入法列表；安装未完成；请重试安装，仍失败时查看服务日志"
LangString FT_ENABLE_FAILED ${LANG_TRADCHINESE} "無法加入目前使用者的輸入法清單；安裝未完成；請重試安裝，仍失敗時查看服務日誌"
LangString FT_ENABLE_FAILED ${LANG_ENGLISH} "Cannot add FreeType to this user's input method list； setup is incomplete； run setup again； if it fails, check the service log"

LangString FT_DISABLE_FAILED ${LANG_SIMPCHINESE} "无法移出当前用户的输入法列表；卸载已停止，程序和用户数据已保留；请运行安装器修复后再卸载"
LangString FT_DISABLE_FAILED ${LANG_TRADCHINESE} "無法移出目前使用者的輸入法清單；解除安裝已停止，程式和使用者資料已保留；請執行安裝程式修復後再解除安裝"
LangString FT_DISABLE_FAILED ${LANG_ENGLISH} "Cannot remove FreeType from this user's input method list； uninstall stopped； the program and user data are kept； run setup to repair FreeType before uninstalling"

LangString FT_UNREGISTER_FAILED ${LANG_SIMPCHINESE} "输入法注销失败或管理员操作已取消；卸载已停止，程序、启动项和用户数据已保留；请重试卸载，输入异常时运行安装器修复"
LangString FT_UNREGISTER_FAILED ${LANG_TRADCHINESE} "輸入法取消註冊失敗或系統管理員操作已取消；解除安裝已停止，程式、啟動項目和使用者資料已保留；請重試解除安裝，輸入異常時執行安裝程式修復"
LangString FT_UNREGISTER_FAILED ${LANG_ENGLISH} "Administrator access was cancelled or input method removal failed； uninstall stopped； the program, startup entry and user data are kept； retry uninstalling； if input fails, run setup to repair FreeType"

LangString FT_RESTORE_FAILED ${LANG_SIMPCHINESE} "输入法注销未确认，输入法列表恢复失败；卸载已停止，程序、启动项和用户数据已保留；请运行安装器修复后再卸载"
LangString FT_RESTORE_FAILED ${LANG_TRADCHINESE} "輸入法取消註冊未確認，輸入法清單還原失敗；解除安裝已停止，程式、啟動項目和使用者資料已保留；請執行安裝程式修復後再解除安裝"
LangString FT_RESTORE_FAILED ${LANG_ENGLISH} "Input method removal was not confirmed and the input method list could not be restored； uninstall stopped； the program, startup entry and user data are kept； run setup to repair FreeType before uninstalling"

; 与已发布服务的 pipe::single_instance 使用同一个名字。保留一个句柄即可让
; 新旧服务的 CreateMutex 都看到 ERROR_ALREADY_EXISTS；不等待/占有互斥体，
; 避免等待仍在运行的服务退出。测试编译时覆盖名字，绝不占用用户的真实实例。
; 卸载会整个删掉下面两个用户目录。测试编译覆盖了服务锁的名字，就必须同时把这两个目录
; 换成测试目录，否则不让编译：测试卸载器绝不能碰到用户真实的设置和学习记录。
!ifdef FREETYPE_SERVER_MUTEX
  !ifndef FREETYPE_USER_DATA
    !error "Test builds must override FREETYPE_USER_DATA and FREETYPE_LOCAL_DATA"
  !endif
  !ifndef FREETYPE_LOCAL_DATA
    !error "Test builds must override FREETYPE_USER_DATA and FREETYPE_LOCAL_DATA"
  !endif
!endif
!ifndef FREETYPE_SERVER_MUTEX
  !define FREETYPE_SERVER_MUTEX "Local\FreeType.Server"
!endif
; 设置、词库、学习记录、同步账号、下载的更新包（服务进程的 data_dir）
!ifndef FREETYPE_USER_DATA
  !define FREETYPE_USER_DATA "$APPDATA\FreeType"
!endif
; 日志和词库快照（服务进程的 local_dir、输入法 DLL 的日志）
!ifndef FREETYPE_LOCAL_DATA
  !define FREETYPE_LOCAL_DATA "$LOCALAPPDATA\FreeType"
!endif
!ifndef FREETYPE_UPDATE_PENDING
  !define FREETYPE_UPDATE_PENDING "${FREETYPE_USER_DATA}\updates\pending.json"
!endif
Var FreeTypeServerGuard

; 安装和卸载共用按安装路径检查的停止步骤，不按进程名结束其他实例。
!macro FreeTypePrepareFunction Prefix
Function ${Prefix}FreeTypePrepare
  ; 必须先保留服务锁再停止进程，消除旧 DLL 在停止与复制之间重新拉起服务的空隙。
  StrCmp $FreeTypeServerGuard "" 0 guarded
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "${FREETYPE_SERVER_MUTEX}") p .r0'
  StrCmp $0 0 guard_failed
  StrCpy $FreeTypeServerGuard $0
  Goto guarded
  guard_failed:
    MessageBox MB_ICONSTOP|MB_OK "$(FT_STOP_FAILED)" /SD IDOK
    SetErrorLevel 2
    Quit
  guarded:
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
  ; customInstall 到这里时，文件、两种架构的组件注册和用户启用都已完成。
  ; 用现有服务锁兼容旧 DLL；退出/取消时系统也会自动回收句柄，不留持久阻断文件。
  StrCmp $FreeTypeServerGuard "" released
  System::Call 'kernel32::CloseHandle(p $FreeTypeServerGuard)'
  StrCpy $FreeTypeServerGuard ""
  released:
  nsExec::ExecToLog /TIMEOUT=60000 '"$INSTDIR\bin\FreeTypeServer.exe" --start-and-check'
  Pop $0
  StrCmp $0 "0" ready
    MessageBox MB_ICONSTOP|MB_OK "$(FT_START_FAILED)" /SD IDOK
    SetErrorLevel 8
    Quit
  ready:
  ; 服务已从本次安装目录启动且词库可用，才结束更新事务。
  Delete "${FREETYPE_UPDATE_PENDING}"
  System::Call 'Kernel32::SetEnvironmentVariable(t "FREETYPE_INSTALL_COMPLETED", t "1")'
  Exec '"$INSTDIR\FreeType.exe" --after-install'
FunctionEnd

!macro customInstall
  InitPluginsDir
  ; 旧版在安装目录留下的默认词库快照（约 100 MB）；现在默认词库改用编译索引，不再读它。
  Delete "$INSTDIR\data\lexicon.tsv.wcache"
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
  ; 卸载不留用户数据（和水杉输入法一样）：设置、导入的词库、学习记录、同步账号、日志和缓存。
  ; 只在输入法已经停用并注销以后删；前面任何一步失败都会退出，数据原样保留。
  ; 覆盖安装和自动更新不经过卸载器，数据不受影响。
  ; 系统给不出用户目录时路径会变成盘符根下的 \FreeType，那种情况不删。
  StrCmp "$APPDATA" "" +2 0
    RMDir /r "${FREETYPE_USER_DATA}"
  StrCmp "$LOCALAPPDATA" "" +2 0
    RMDir /r "${FREETYPE_LOCAL_DATA}"
!macroend
