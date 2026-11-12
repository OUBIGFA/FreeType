; FreeType 安装器钩子（mygo 生成的 NSIS 安装器会包含本文件）。
;
; 安装器按当前用户安装、不要管理员权限；只有注册系统输入法这一步用 runas 单独提权，
; 由 bin\FreeTypeServer.exe --register-tsf 把输入法 DLL 放进 Program Files 并注册。

!macro customInit
  ; 升级前让旧的服务进程存好学到的词再退出，关掉托盘程序，免得文件被占用
  IfFileExists "$INSTDIR\bin\FreeTypeServer.exe" 0 +2
    nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --stop'
  nsExec::Exec 'taskkill /F /IM FreeTypeServer.exe'
  nsExec::Exec 'taskkill /F /IM FreeType.exe'
!macroend

!macro customInstall
  Delete "$TEMP\freetype-register.ok"
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" '--register-tsf "$INSTDIR\bin" --marker "$TEMP\freetype-register.ok"' SW_HIDE
  IfFileExists "$TEMP\freetype-register.ok" registered 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "FreeType 输入法没有注册到系统（可能取消了管理员授权）。再运行一次安装程序即可重试。" /SD IDOK
  registered:
  Delete "$TEMP\freetype-register.ok"
  ; 加进当前用户的输入法列表，开机时放托盘图标并启动输入法服务
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --enable'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType" '"$INSTDIR\FreeType.exe" --background'
  Exec '"$INSTDIR\FreeType.exe" --background'
!macroend

!macro customUnInstall
  nsExec::Exec 'taskkill /F /IM FreeType.exe'
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --stop'
  nsExec::Exec '"$INSTDIR\bin\FreeTypeServer.exe" --disable'
  ExecShellWait "runas" "$INSTDIR\bin\FreeTypeServer.exe" "--unregister-tsf" SW_HIDE
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "FreeType"
!macroend
