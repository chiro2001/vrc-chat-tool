//! SteamVR Dashboard overlay (Phase 5).
//! Creates a dashboard tab in the SteamVR system menu.
//! Uses openvr-sys2 for CreateDashboardOverlay (not wrapped by openvr 0.9).

use crate::state::OverlayState;

/// Create and manage dashboard overlay.
/// Returns (dashboard_handle, thumbnail_handle).
pub fn create_dashboard_overlay(
    ctx: &openvr::Context,
) -> anyhow::Result<(
    openvr::sys::VROverlayHandle_t,
    openvr::sys::VROverlayHandle_t,
)> {
    // openvr 0.9 does not wrap CreateDashboardOverlay.
    // Use openvr-sys2 FFI to call it directly.
    //
    // C FFI signature:
    //   EVROverlayError CreateDashboardOverlay(
    //     const char *pchOverlayKey,
    //     const char *pchOverlayName,
    //     VROverlayHandle_t *pMainHandle,
    //     VROverlayHandle_t *pThumbnailHandle
    //   );
    //
    // The overlay key must match the one in vrc-chat-tool.vrmanifest.
    // The overlay name appears in the SteamVR dashboard tab.

    let overlay_key = std::ffi::CString::new("com.vrcchattool.dashboard")?;
    let overlay_name = std::ffi::CString::new("VRC Chat Tool")?;

    let mut main_handle: openvr::sys::VROverlayHandle_t = 0;
    let mut thumb_handle: openvr::sys::VROverlayHandle_t = 0;

    // Get the raw IVROverlay pointer from openvr context
    // (openvr 0.9 exposes this via the Context interface)
    let ovr = ctx.overlay()?;

    // Call via openvr internal FFI
    // In production: cast ovr's internal pointer and call CreateDashboardOverlay
    // For now: scaffold showing the intended API call
    tracing::info!(
        "Dashboard overlay scaffold: key={} name={}",
        overlay_key.to_str().unwrap_or("?"),
        overlay_name.to_str().unwrap_or("?")
    );

    // Placeholder: in production, this would call:
    // unsafe {
    //     let ovr_raw = ovr.as_raw(); // need openvr-sys2 for this
    //     (openvr_sys::VR_IVROverlay_015_CreateDashboardOverlay)(
    //         ovr_raw,
    //         overlay_key.as_ptr(),
    //         overlay_name.as_ptr(),
    //         &mut main_handle,
    //         &mut thumb_handle,
    //     );
    // }

    let _ = ovr;
    anyhow::bail!("CreateDashboardOverlay requires openvr-sys2 FFI binding (not yet implemented)");
}

/// Render the dashboard overlay content (simplified control panel).
/// Called from the main event loop when dashboard tab is active.
pub fn render_dashboard(
    ui: &imgui::Ui,
    state: &OverlayState,
) {
    imgui::Window::new("VRC Chat Tool")
        .size([400.0, 300.0], imgui::Condition::FirstUseEver)
        .build(ui, || {
            ui.text("语音识别控制面板");
            ui.separator();

            // Status
            let status_color = match state.status.as_str() {
                "recording" => [0.9, 0.2, 0.2, 1.0],
                "recognizing" => [1.0, 0.7, 0.2, 1.0],
                _ => [0.3, 0.8, 0.3, 1.0],
            };
            ui.text_colored(status_color, format!("● {}", status_label(&state.status)));

            // Volume meter
            ui.text(format!("麦克风音量: {:.0}%", state.volume * 100.0));
            let _token = ui.begin_disabled(true);
            ui.progress_bar(state.volume, [200.0, 16.0], "");
            drop(_token);

            // Model info
            ui.text(format!("引擎: {}", state.model));

            // Current recognition
            if !state.current_text.is_empty() {
                ui.separator();
                ui.text("实时识别:");
                ui.text_wrapped(&state.current_text);
            }

            // Last sentence
            if !state.last_sentence.is_empty() {
                ui.separator();
                ui.text("上句结果:");
                ui.text_wrapped(&state.last_sentence);
            }

            ui.separator();

            // Control hint
            ui.text_colored(
                [0.6, 0.6, 0.6, 1.0],
                "提示: 双击左手扳机 — 开始/停止录音",
            );
        });
}

fn status_label(status: &str) -> &str {
    match status {
        "recording" => "录音中",
        "recognizing" => "识别中",
        "error" => "错误",
        _ => "就绪",
    }
}
