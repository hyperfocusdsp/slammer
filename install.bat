@echo off
setlocal EnableDelayedExpansion

:: Install Niner VST3, CLAP, and standalone into standard locations.
:: Expects to be run from the extracted release folder (same folder as
:: niner.vst3, niner.clap, and niner-standalone.exe).
:: Run as Administrator — VST3/CLAP install paths require elevation.

set "SCRIPT_DIR=%~dp0"
set "VST3_BUNDLE=%SCRIPT_DIR%niner.vst3"
set "CLAP_BUNDLE=%SCRIPT_DIR%niner.clap"
set "STANDALONE=%SCRIPT_DIR%niner-standalone.exe"

if not exist "%VST3_BUNDLE%" if not exist "%CLAP_BUNDLE%" (
    echo Error: no niner.vst3 or niner.clap found in %SCRIPT_DIR%
    echo Make sure install.bat is in the same folder as the extracted release.
    pause
    exit /b 1
)

echo Installing Niner...

:: VST3
if exist "%VST3_BUNDLE%" (
    set "VST3_DEST=%CommonProgramFiles%\VST3"
    if not exist "!VST3_DEST!" mkdir "!VST3_DEST!"
    if exist "!VST3_DEST!\niner.vst3" rmdir /s /q "!VST3_DEST!\niner.vst3"
    xcopy /e /i /y "%VST3_BUNDLE%" "!VST3_DEST!\niner.vst3" >nul
    echo   VST3       -^> !VST3_DEST!\niner.vst3
)

:: CLAP
if exist "%CLAP_BUNDLE%" (
    set "CLAP_DEST=%CommonProgramFiles%\CLAP"
    if not exist "!CLAP_DEST!" mkdir "!CLAP_DEST!"
    if exist "!CLAP_DEST!\niner.clap" rmdir /s /q "!CLAP_DEST!\niner.clap"
    xcopy /e /i /y "%CLAP_BUNDLE%" "!CLAP_DEST!\niner.clap" >nul
    echo   CLAP       -^> !CLAP_DEST!\niner.clap
)

:: Standalone
if exist "%STANDALONE%" (
    set "BIN_DIR=%LocalAppData%\Niner"
    if not exist "!BIN_DIR!" mkdir "!BIN_DIR!"
    copy /y "%STANDALONE%" "!BIN_DIR!\niner-standalone.exe" >nul
    echo   Standalone -^> !BIN_DIR!\niner-standalone.exe
)

echo.
echo Done! Rescan plugins in your DAW to find Niner.
echo Note: If VST3/CLAP install failed, re-run this script as Administrator.
pause
