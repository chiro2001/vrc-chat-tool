@echo off
set LIBCLANG_PATH=D:\Software\LLVM\clang+llvm-18.1.8-x86_64-pc-windows-msvc\bin
cd /d D:\Projects\vrc-chat-tool

echo === Building frontend ===
call npm run build

echo === Building Rust ===
cargo build -p vrc-chat-tool

echo === Starting Tauri dev ===
call npm run tauri dev
