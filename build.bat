@echo off
set LIBCLANG_PATH=D:\Software\LLVM\clang+llvm-18.1.8-x86_64-pc-windows-msvc\bin
cd /d D:\Projects\vrc-chat-tool

echo === Building frontend ===
call npm run build
if %ERRORLEVEL% neq 0 goto :error

echo === Building Rust ===
cargo build -p vrc-chat-tool
if %ERRORLEVEL% neq 0 goto :error

echo === Done ===
exit /b 0

:error
echo BUILD FAILED
pause
exit /b 1
