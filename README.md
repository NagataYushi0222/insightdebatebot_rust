# InsightDebateBot (Rust Edition)

InsightDebateBot は、Discord のボイスチャットでの議論を録音・分析するためのボイスチャット分析 Bot です。
Rust で記述されており、高速かつ効率的に動作します。Gemini AI を使用して、議論の内容をリアルタイムに近い感覚で分析・要約します。

## 主な機能

- **ボイスチャット録音**: Opus 音声ストリームを直接受信し、高品質に録音します。
- **AI 分析 (Gemini)**: 設定された間隔で議論の内容を Gemini AI に送信し、分析レポートを作成します。
- **分析モード**:
  - `debate`: 議論モード（論点の整理、賛否の分析など）
  - `summary`: 要約モード（会話の要約、決定事項の整理など）
- **コマンド操作**: Discord のスラッシュコマンドで簡単に操作できます。

## 必要な環境

- Rust (最新の安定版)
- Discord Bot Token
- Google Gemini API Key
- FFmpeg (音声処理に推奨)

## インストールとセットアップ

### Dockerを使用する場合（推奨）

1. **リポジトリのクローン**
   ```bash
   git clone https://github.com/NagataYushi0222/insightdebatebot_rust.git
   cd insightdebatebot_rust
   ```

2. **環境設定**
   `.env.template` を `.env` にコピーし、必要な情報を入力してください。
   ```bash
   cp .env.template .env
   ```

3. **起動**
   ```bash
   docker compose up -d
   ```

### 手動でインストールする場合

1. **リポジトリのクローン**
   ```bash
   git clone https://github.com/NagataYushi0222/insightdebatebot_rust.git
   cd insightdebatebot_rust
   ```

2. **依存関係のビルド**
   ```bash
   cargo build --release
   ```

3. **環境設定**
   `.env.template` を `.env` にコピーし、必要な情報を入力してください。
   ```bash
   cp .env.template .env
   ```

   **`.env` の設定項目:**
   - `DISCORD_TOKEN`: Discord Developer Portal で取得した Bot トークン
   - `GEMINI_API_KEY`: Google AI Studio で取得した API キー
   - `GUILD_ID`: (任意) 開発用。コマンドを即時反映させたいサーバーのIDを指定
   - `TEMP_AUDIO_DIR`: (任意) 一時音声ファイルを保存するディレクトリ（デフォルト: `temp_audio`）
   - `RECORDING_INTERVAL`: (任意) デフォルトの分析間隔（秒）（デフォルト: 300）

## 使い方

Bot を起動します。
```bash
cargo run --release
```

### スラッシュコマンド

| コマンド | 説明 |
| --- | --- |
| `/analyze_start` | ボイスチャットの録音と分析を開始します。実行者が参加しているボイスチャンネルに参加します。 |
| `/analyze_stop` | 録音を停止し、最終レポートを作成してから退出します。 |
| `/analyze_now` | 分析間隔を待たずに、現在の議論内容で即座にレポートを作成します。 |
| `/settings set_mode <mode>` | 分析モードを変更します。<br>`debate`: 議論分析, `summary`: 要約 |
| `/settings set_interval <seconds>` | 分析間隔（秒）を変更します。（60秒〜3600秒） |

## 開発

```bash
# 開発モードで実行（ログレベル debug）
RUST_LOG=debug cargo run
```

## ライセンス

[MIT License](LICENSE)
