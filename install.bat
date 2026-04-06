@echo off
setlocal EnableDelayedExpansion

:: Install Slammer VST3, CLAP, and standalone into standard locations.
:: Expects to be run from the extracted release folder (same folder as
:: slammer.vst3, slammer.clap, and slammer-standalone.exe).
:: Run as Administrator — VST3/CLAP install paths require elevation.

set "SCRIPT_DIR=%~dp0"
set "VST3_BUNDLE=%SCRIPT_DIR%slammer.vst3"
set "CLAP_BUNDLE=%SCRIPT_DIR%slammer.clap"
set "STANDALONE=%SCRIPT_DIR%slammer-standalone.exe"

if not exist "%VST3_BUNDLE%" if not exist "%CLAP_BUNDLE%" (
    echo Error: no slammer.vst3 or slammer.clap found in %SCRIPT_DIR%
    echo Make sure install.bat is in the same folder as the extracted release.
    pause
    exit /b 1
)

echo Installing Slammer...

:: VST3
if exist "%VST3_BUNDLE%" (
    set "VST3_DEST=%CommonProgramFiles%\VST3"
    if not exist "!VST3_DEST!" mkdir "!VST3_DEST!"
    if exist "!VST3_DEST!\slammer.vst3" rmdir /s /q "!VST3_DEST!\slammer.vst3"
    xcopy /e /i /y "%VST3_BUNDLE%" "!VST3_DEST!\slammer.vst3" >nul
    echo   VST3       -^> !VST3_DEST!\slammer.vst3
)

:: CLAP
if exist "%CLAP_BUNDLE%" (
    set "CLAP_DEST=%CommonProgramFiles%\CLAP"
    if not exist "!CLAP_DEST!" mkdir "!CLAP_DEST!"
    if exist "!CLAP_DEST!\slammer.clap" rmdir /s /q "!CLAP_DEST!\slammer.clap"
    xcopy /e /i /y "%CLAP_BUNDLE%" "!CLAP_DEST!\slammer.clap" >nul
    echo   CLAP       -^> !CLAP_DEST!\slammer.clap
)

:: Standalone
if exist "%STANDALONE%" (
    set "BIN_DIR=%LocalAppData%\Slammer"
    if not exist "!BIN_DIR!" mkdir "!BIN_DIR!"
    copy /y "%STANDALONE%" "!BIN_DIR!\slammer-standalone.exe" >nul
    echo   Standalone -^> !BIN_DIR!\slammer-standalone.exe
)

echo.
echo Done! Rescan plugins in your DAW to find Slammer.
echo Note: If VST3/CLAP install failed, re-run this script as Administrator.
pause
