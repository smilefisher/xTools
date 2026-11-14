@echo off
rem xTools MFT 验证脚本
rem 用法：右键本文件 ->「以管理员身份运行」
rem 结果会写到 %LOCALAPPDATA%\xtools\logs\probe-mft.txt，AI 下次会自动读取
setlocal enabledelayedexpansion

set "LOG_DIR=%LOCALAPPDATA%\xtools\logs"
set "OUT=%LOG_DIR%\probe-mft.txt"
if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"

set "EXE="
if exist "%LOCALAPPDATA%\xtools-run\xtools.exe" set "EXE=%LOCALAPPDATA%\xtools-run\xtools.exe"
if not defined EXE if exist "%LOCALAPPDATA%\Programs\xTools\xTools.exe" set "EXE=%LOCALAPPDATA%\Programs\xTools\xTools.exe"
if not defined EXE if exist "%~dp0..\target\release\xtools.exe" set "EXE=%~dp0..\target\release\xtools.exe"

if not defined EXE (
  echo 找不到 xtools.exe，请确认已安装或已构建 > "%OUT%"
  type "%OUT%"
  pause
  exit /b 1
)

echo ==== %DATE% %TIME% ==== > "%OUT%"
echo exe: %EXE% >> "%OUT%"
"%EXE%" --probe-mft >> "%OUT%" 2>&1

echo.
echo 结果已写入：%OUT%
echo ----------------------------------------
type "%OUT%"
echo ----------------------------------------
pause
