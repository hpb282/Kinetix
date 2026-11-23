@echo off
setlocal
cd /d "%~dp0"
where pythonw >nul 2>nul
if not errorlevel 1 (
  start "" pythonw "%~dp0Launch Kinetix.pyw"
  exit /b 0
)
python "%~dp0run_kinetix.py"
exit /b %errorlevel%
