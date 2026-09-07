@echo off
title AntiKarbit Waifu Claimer App
cd /d "%~dp0"

echo ============================================================
echo      ANTIKARBIT: WAIFU CLAIMER DESKTOP DASHBOARD
echo ============================================================
echo * Menjalankan server aplikasi hemat memori (< 65 MB)...
echo * Membuka antarmuka aplikasi di http://localhost:8080
echo ============================================================
echo.

if not exist ".venv\Scripts\python.exe" (
    echo [ERROR] Virtual environment tidak ditemukan di .venv!
    pause
    exit /b 1
)

".venv\Scripts\python.exe" app.py
pause
