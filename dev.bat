@echo off
set LIBCLANG_PATH=D:\Software\LLVM\clang+llvm-18.1.8-x86_64-pc-windows-msvc\bin
cd /d D:\Projects\vrc-chat-tool

echo === Building (tauri build) ===
call npm run tauri build

echo === Building VR HUD ===
cargo build -p vr-hud

echo === Starting Tauri dev ===
call npm run tauri dev
