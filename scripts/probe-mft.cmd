@echo off
rem xTools MFT + USN check - double click, accept UAC
setlocal

net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
  exit /b
)

set "LOG_DIR=%LOCALAPPDATA%\xtools\logs"
set "OUT=%LOG_DIR%\probe-mft.txt"
if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"

set "EXE=%LOCALAPPDATA%\xtools-run\xtools.exe"
if not exist "%EXE%" set "EXE=%LOCALAPPDATA%\Programs\xTools\xTools.exe"
if not exist "%EXE%" set "EXE=%~dp0xtools.exe"
if not exist "%EXE%" (
  echo xtools.exe not found > "%OUT%"
  type "%OUT%"
  pause
  exit /b 1
)

echo ==== %DATE% %TIME% ==== > "%OUT%"
echo exe: %EXE% >> "%OUT%"
"%EXE%" --probe-mft >> "%OUT%" 2>&1
echo. >> "%OUT%"
"%EXE%" --probe-usn >> "%OUT%" 2>&1

echo.
echo ---- %OUT% ----
type "%OUT%"
echo --------------------------------
pause