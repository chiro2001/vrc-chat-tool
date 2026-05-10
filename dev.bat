@echo off
set LIBCLANG_PATH=D:\Software\LLVM\clang+llvm-18.1.8-x86_64-pc-windows-msvc\bin
cd /d D:\Projects\vrc-chat-tool
call npm run tauri dev
