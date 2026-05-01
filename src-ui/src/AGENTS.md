# src-ui/src/ — React 19 Frontend

## OVERVIEW
Single-page desktop UI built with React 19 + TypeScript. Communicates with Rust backend via Tauri v1 `invoke()` and `listen()`.

## STRUCTURE
```
src-ui/src/
├── App.tsx              # Main component (700+ lines — too large)
├── App.css              # Dark theme styles
├── main.tsx             # React DOM render entry
├── i18n.ts              # zh-CN / en translation maps
└── vite-env.d.ts        # Vite type declarations
```

## WHERE TO LOOK
| Task | File | Notes |
|------|------|-------|
| Tauri commands | App.tsx | `invoke("command_name", args)` → `Promise<T>` |
| Tauri events | App.tsx | `listen("event-name", callback)` in `useEffect` |
| State management | App.tsx | `useState` for all state (no context/redux) |
| Settings modal | App.tsx | `showConfigModal` state + render block |
| Test recording | App.tsx | `showTestModal` state + recording test section |
| History display | App.tsx | `recognitionHistory` state + pagination |
| Volume meter | App.tsx | Real-time `volume-update` event handler |
| Trigger echo | App.tsx | `triggerHeardText` state + `trigger-heard` event |
| STT status | App.tsx | `sttStatus` state + `trigger-stt-status` event |
| OSC toggle | App.tsx | `config.osc_enabled` checkbox in settings |
| Provider select | App.tsx | Dropdown: tencent / local / local_embedded |
| i18n strings | i18n.ts | Add key to both `zh` and `en` maps, use `useTranslation()` |

## CONVENTIONS
- **Single component**: App.tsx is the only component — extract subcomponents for maintainability
- **Tauri API**: `import { invoke } from '@tauri-apps/api/tauri'`, `import { listen } from '@tauri-apps/api/event'`
- **TypeScript strict**: All Tauri command payloads have typed interfaces
- **CSS**: Plain CSS (no CSS modules, Tailwind, or styled-components)
- **Dark theme**: Default dark background (#1a1a2e), accent (#4fc3f7)
- **i18n**: `t("key")` helper function, keys in `i18n.ts`

## ANTI-PATTERNS
- **NEVER** use `any` type for Tauri command results — define interface
- **NEVER** mutate `config` state directly — use `setConfig(prev => ({...prev, [key]: value}))`
