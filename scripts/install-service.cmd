@echo off
rem 安装 xTools 索引服务（计划任务）
rem 双击运行即可：脚本会自己申请管理员权限
setlocal

net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
  exit /b
)

set "EXE=%LOCALAPPDATA%\xtools-run\xtools.exe"
if not exist "%EXE%" set "EXE=%LOCALAPPDATA%\Programs\xTools\xTools.exe"
if not exist "%EXE%" (
  echo 找不到 xtools.exe
  pause
  exit /b 1
)

"%EXE%" --install-index-service
echo.
pause
