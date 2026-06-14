<div align="center">

<img src="docs/screenshots/icon.png" width="116" alt="EmulStick Desktop 圖示" />

# EmulStick Desktop

[English](README.md) · **繁體中文**

**以無線方式操控另一台電腦的鍵盤與滑鼠 — 桌面版的低功耗藍牙 HID KVM。**

[![platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows-4c6ef5?style=flat-square)](#環境需求)
[![Tauri](https://img.shields.io/badge/Tauri-2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Svelte](https://img.shields.io/badge/Svelte-5-ff3e00?style=flat-square&logo=svelte&logoColor=white)](https://svelte.dev)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?style=flat-square&logo=rust&logoColor=white)](https://rustup.rs)
[![transport](https://img.shields.io/badge/transport-BLE%20HID-6d28d9?style=flat-square)](docs/ble-protocol.md)
[![release](https://img.shields.io/github/v/release/Crazycurly/EmulStick-GUI?style=flat-square&color=2ea043)](https://github.com/Crazycurly/EmulStick-GUI/releases/latest)

</div>

**[EmulStick](https://emulstick.com)** 低功耗藍牙 HID 模擬器的操作端主控台。EmulStick 是一款隨插即用的 USB 2.0 接收器，目標電腦會把它辨識為標準的 USB 鍵盤／滑鼠／搖桿，並透過低功耗藍牙（BLE）接收輸入（免驅動、免配對）。本程式是以 Tauri 2 + Svelte + Rust 打造的原生桌面應用程式，會與接收器連線，把你真實的鍵盤與滑鼠轉送到目標電腦 —— **包含瀏覽器工具永遠攔截不到的系統保留快速鍵**（`⌘Tab`、`Win`、`Ctrl`+`Alt`+`Del`、`⌘`+`Space`…）。再用 USB 擷取卡把目標電腦的 HDMI 畫面接進來，就成了完整的 PiKVM 式遠端主控台 —— 目標端不必安裝任何代理程式，只會看到一個普通的 USB 鍵盤／滑鼠。

> EmulStick 接收器為市售產品 —— 硬體與購買資訊請見 **[emulstick.com](https://emulstick.com)**。本儲存庫是它的桌面操作端程式。

<div align="center">

| 精簡主控台 | KVM／視訊模式 |
| :-: | :-: |
| <img src="docs/screenshots/compact.png" width="300" alt="精簡的已連線畫面" /> | <img src="docs/screenshots/kvm.png" width="420" alt="帶即時 HDMI 畫面的 KVM 模式" /> |

</div>

## 下載

**[⬇ 下載最新發行版本](https://github.com/Crazycurly/EmulStick-GUI/releases/latest)** —— macOS（Apple Silicon）。

開啟 `.dmg`，把 **EmulStick** 拖曳到「應用程式」資料夾。

> [!NOTE]
> 此版本以開發者身分簽署，但**未經 Apple 公證**，因此首次開啟時 macOS Gatekeeper 會跳出警告。請在 App 上按右鍵 →「**打開**」一次，或清除隔離屬性：
> ```bash
> xattr -dr com.apple.quarantine /Applications/EmulStick.app
> ```
> 接著授予 **輔助使用** 權限（見 [macOS 權限](#macos-權限)），輸入轉送才能運作。想自行建置？請見 [開發](#開發)。

## 功能特色

- 🔗 **藍牙連線啟動** —— 掃描、連線、讀取裝置資訊服務（DIS）、寫入 F801/F803 HID 特徵值、訂閱鍵盤 LED 通知。
- ⌨️🖱️ **全域輸入攔截** —— 透過 `rdev::grab` 掛鉤在作業系統層攔截事件，連系統保留快速鍵都會送往*目標電腦*而非你的機器。滑鼠採用相對 HID 位移，並凍結本機游標。
- 🎛️ **分頻道直通** —— 鍵盤、滑鼠、視訊可獨立轉送；每項選擇都會在重開後保留。
- 🖥️ **HDMI 視訊／KVM 模式** —— 透過任何 UVC 擷取卡提供 PiKVM 式全視窗畫面，支援即時切換來源與熱插拔復原。
- 🔒 **鎖定模式** —— 啟用後即轉送所有輸入；**`Ctrl`+`Alt`/`⌥`** 為永遠有效的緊急解鎖。連線中斷、寫入失敗或變更頻道時，都會回到安全的「全部放開」狀態，確保不會有按鍵卡在目標電腦上。
- ♻️ **記住裝置 + 自動重連** —— 記住上次連線的接收器，斷線後以退避（backoff）方式自動重連。
- ⚡ **為手感調校** —— 約 1 kHz 的輸入會先合併，再於接近 BLE 連線間隔時送出，讓快速移動不會塞爆連線（見 [`docs/plan.md`](docs/plan.md) §6.3）。

## 運作原理

將**控制平面**（Svelte 前端）與**資料平面**（Rust 後端）分離，讓高頻輸入完全不經過 JSON IPC 橋接。

```
 你的鍵盤/滑鼠 ─▶ rdev::grab（攔截）─▶ HID 編碼 ─▶ 合併/送出 ─▶ BLE GATT 寫入 ─▶ EmulStick 接收器 ─▶ USB ─▶ 目標電腦
                                                                          目標 HDMI ─▶ 擷取卡 ─▶ getUserMedia ─▶ KVM 畫面
```

- **前端**（`src/`）—— 掃描/連線/狀態介面、直通開關、KVM 視訊、錯誤提示。僅處理低頻指令／事件。
- **後端**（`src-tauri/src/`）：
  - `protocol/` —— HID 報告編碼器（鍵盤 8 B／滑鼠 6 B）、`rdev::Key → HID usage` 對應表，以及標準 BLE UUID。與硬體無關，並針對 [`docs/protocol.md`](docs/protocol.md) 中的範例做逐位元組單元測試。
  - `ble/` —— 以 `btleplug` 進行掃描/連線（含連線逾時）、讀取裝置資訊、對 F801/F803 寫入（write-without-response）、LED 通知。
  - `input/` —— `rdev::grab` 執行緒、鎖定模式狀態機、相對游標擷取，以及 §6.3 的滑鼠合併。
  - `ipc/` —— Tauri 指令與事件。
  - `state.rs` —— 直通旗標與鎖定狀態。

完整的工程設計請見 [`docs/plan.md`](docs/plan.md)；傳輸格式請見 [`docs/protocol.md`](docs/protocol.md) / [`docs/ble-protocol.md`](docs/ble-protocol.md) —— 兩者皆源自官方 [EmulStick BLE 通訊協定規格（v0.93）](https://www.emulstick.com/files/emulstick_ble_v0.93.pdf)。

## 開發狀態

里程碑 **M1–M5 已完成**（藍牙啟動 → 輸入管線 → 直通與介面 → 視訊 → 強化），另含一輪程式碼審查後的強化。本程式已能連線實體硬體、擷取並轉送鍵盤/滑鼠、記住並自動重連上次的裝置，並顯示即時 HDMI KVM 畫面。里程碑清單見 [`docs/plan.md`](docs/plan.md) §12。

## 環境需求

- [Rust](https://rustup.rs)（stable）與對應平台的 [Tauri 環境需求](https://tauri.app/start/prerequisites/)。
- Node.js 18+ 與 npm。
- 一支 [EmulStick](https://emulstick.com) 低功耗藍牙 HID 接收器（插在**目標電腦**上）—— 可於上述官方商店購買 —— 若要使用視訊，還需一張 UVC HDMI 擷取卡。

## 開發

```bash
npm install            # 前端相依套件 + Tauri CLI
npm run tauri dev      # 執行 App（同時啟動 Vite 與 Rust 後端）
```

### 常用指令

```bash
npm run build                                     # 建置 Svelte 前端
npm run check                                     # svelte-check（型別檢查）
cargo test --manifest-path src-tauri/Cargo.toml   # 通訊協定逐位元組測試
npx tauri icon app-icon.png                       # 由來源圖片重新產生圖示
```

## macOS 權限

全域輸入攔截需要在 **「系統設定」→「隱私權與安全性」→「輔助使用」**（可能還需要 **「輸入監控」**）中授權給本 App。若在尚未授權的情況下要求鎖定模式，App 會跳出 macOS 的授權對話框，並顯示一張內建引導卡片，附有 **「開啟設定」** 捷徑；當你切回視窗時會自動重新檢查。藍牙則會在首次使用時透過 `NSBluetoothAlwaysUsageDescription` 提示。

未簽署的版本每次更新都會失去「輔助使用」授權。開發迭代時，`cargo run` 的 runner 會以穩定身分重新簽署（見 [`scripts/sign-and-run.sh`](scripts/sign-and-run.sh)）；若要產生可散布、已公證的發行版本，請見 [`docs/release.md`](docs/release.md)。
