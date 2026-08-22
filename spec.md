# rview — Rust/egui 画像ビューワー仕様書

WPF 製 `view` を Rust + egui で移植した画像ビューワーの仕様。

---

## プロジェクト概要

`rview` は egui/eframe ベースのクロスプラットフォーム画像ビューワー。
ドラッグ＆ドロップで画像ファイル/フォルダを開き、キーボード・マウスホイールで操作できる。

- **ターゲット**: Linux / Windows / macOS
- **実行バイナリ**: `rview`（コンソールウィンドウなし）

---

## Tech Stack

| 項目 | 値 |
|---|---|
| 言語 | Rust (edition 2021) |
| UI フレームワーク | [egui](https://github.com/emilk/egui) + [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) |
| 画像デコード | `image` crate |
| 自然順ソート | 純 Rust 実装（後述） |
| 非同期 | 使用しない（フレーム駆動で完結） |

### Cargo.toml（主要依存）

```toml
[package]
name = "rview"
version = "0.1.0"
edition = "2021"
license = "MIT"

[[bin]]
name = "rview"
path = "src/main.rs"

[dependencies]
eframe = { version = "0.31", features = ["default"] }
egui = "0.31"
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "bmp", "gif", "tiff", "webp"] }

[profile.release]
opt-level = 3
strip = true
```

---

## リポジトリ構造

```
rview/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE              ← MIT
├── spec.md
├── doc/
│   └── port.md          ← 移植元 WPF アプリの仕様
├── assets/              ← フォント等（リポジトリには含めない）
└── src/
    └── main.rs          ← 全アプリケーションロジック
```

`assets/` はサイズが大きいためリポジトリに含めていない。
日本語表示用フォントの配置方法は README を参照。

---

## アーキテクチャ概要

**単一構造体設計**: `App` 構造体に全ロジックを集約。
egui は即時モード GUI のため、フレームごとに `update()` が呼ばれる。
タイマーは `std::time::Instant` でフレーム内比較により代替する。

### `App` 構造体フィールド

| フィールド | 型 | 用途 |
|---|---|---|
| `current_dir` | `PathBuf` | 作業ディレクトリ（初期値 `./`） |
| `current_file` | `Option<PathBuf>` | 現在表示中のファイルパス |
| `current_texture` | `Option<egui::TextureHandle>` | GPU にアップロード済みの画像テクスチャ |
| `current_image_size` | `Option<(u32, u32)>` | ロード済み画像の幅・高さ（ピクセル） |
| `images` | `Vec<PathBuf>` | 現在ディレクトリの画像ファイル一覧 |
| `current_index` | `usize` | `images` 内の現在インデックス |
| `is_sliding` | `bool` | スライドショー稼働中フラグ |
| `slide_last` | `Instant` | 最後にスライドした時刻 |
| `rotation_angle` | `f32` | 現在の回転角度（度、0–359） |
| `show_key_overlay` | `bool` | キーオーバーレイ表示フラグ |
| `key_overlay_text` | `String` | オーバーレイに表示するテキスト |
| `key_overlay_hide_at` | `Option<Instant>` | オーバーレイを非表示にする時刻 |

### 定数

```rust
const SLIDE_SHOW_INTERVAL_MS: u64 = 3000;
const KEY_OVERLAY_SHOW_MS:    u64 = 1000;
const PAGE_JUMP_COUNT:        usize = 10;
const SUPPORTED_EXTENSIONS:   &[&str] = &["jpeg", "jpg", "bmp", "gif", "tiff", "tif", "png", "webp"];
```

---

## 実装詳細

### エントリポイント (`main.rs`)

```rust
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        drag_and_drop_support: true,
        ..Default::default()
    };
    eframe::run_native("rview", options, Box::new(|_cc| Ok(Box::new(App::new()))))
}
```

`eframe::App` トレイトを `App` に実装し、`update()` 内で描画・入力処理を行う。

---

### 画像ロード・ナビゲーション

| メソッド | 役割 |
|---|---|
| `list_up_images(&mut self, dir: &Path)` | ディレクトリから対応拡張子ファイルを列挙し `images` に格納 |
| `init_images(&mut self, dir: &Path, file: &Path, ctx: &egui::Context)` | `list_up_images` → `natural_sort` → `load_current_image` |
| `load_current_image(&mut self, ctx: &egui::Context)` | `current_index` のファイルをデコードしてテクスチャ生成 |
| `shift_image(&mut self, is_next: bool, ctx: &egui::Context)` | 前後ナビゲーション（端でループ） |
| `load_first_image(&mut self, ctx: &egui::Context)` | インデックス 0 へジャンプ |
| `load_last_image(&mut self, ctx: &egui::Context)` | 最終インデックスへジャンプ |
| `load_pgup_image(&mut self, ctx: &egui::Context)` | `PAGE_JUMP_COUNT` 枚戻る（0 でクランプ） |
| `load_pgdn_image(&mut self, ctx: &egui::Context)` | `PAGE_JUMP_COUNT` 枚進む（最大でクランプ） |
| `set_current_index_by_path(&mut self, path: &Path)` | パスで `images` を検索し `current_index` を設定 |

#### 画像デコード処理

```
image::open(path)
  → image::DynamicImage
  → .to_rgba8()
  → egui::ColorImage::from_rgba_unmultiplied(...)
  → ctx.load_texture(name, color_image, TextureOptions::LINEAR)
  → current_texture に格納
```

テクスチャは毎回再生成（前のテクスチャは自動ドロップ）。

---

### スライドショー

| メソッド | 役割 |
|---|---|
| `slide_show(&mut self, active: bool)` | `is_sliding` フラグと `slide_last` を設定 |

`update()` 内で毎フレーム以下を確認する:

```rust
if self.is_sliding && self.slide_last.elapsed() >= Duration::from_millis(SLIDE_SHOW_INTERVAL_MS) {
    self.shift_image(true, ctx);
    self.slide_last = Instant::now();
}
```

スライドショー中はウィンドウタイトルに `[Slide]` を付加し、`ctx.request_repaint_after(...)` で定期再描画を要求する。

---

### 回転

| メソッド | 役割 |
|---|---|
| `rotate_image(&mut self, angle: f32)` | 絶対角度で `rotation_angle` を設定 |
| `rotate_image_add(&mut self, delta: f32)` | 現在角度に加算（0–359 範囲に正規化） |

描画時に `egui::Image` の `rotate()` メソッドまたは `egui::Sense` + transform で回転を適用する。

---

### PNG 保存（回転適用）

| メソッド | 役割 |
|---|---|
| `save_png(&self)` | 現在の回転角で PNG として上書き保存 |

```
image::open(current_file)
  → DynamicImage
  → rotate に応じて rotate90 / rotate180 / rotate270 / そのまま
  → save(path) で PNG として保存
```

**PNG ファイルのみ対象**（拡張子チェックを呼び出し元で実施）。

---

### ファイル操作

| メソッド | 役割 |
|---|---|
| `rename_file(&mut self, new_name: &str)` | ファイルリネーム後 `images` リストを更新し `current_index` を再設定 |

---

### ユーティリティ

| メソッド | 役割 |
|---|---|
| `window_title(&self) -> String` | `filename[dir](WxH) N/Total [-Slide] [-Rotated: X°]` 形式の文字列生成 |
| `natural_sort(&mut self)` | ファイル名を自然順ソート（後述） |
| `show_key(&mut self, text: &str)` | キーオーバーレイテキストを設定し `key_overlay_hide_at` を更新 |

#### ウィンドウタイトル形式

```
photo.jpg[/home/user/pics](1920x1080) 3/42 [-Slide] [-Rotated: 90°]
```

---

### 自然順ソート（`natural_sort`）

WPF 版は Windows の `StrCmpLogicalW` を使用していたが、Rust 版では純粋な Rust 実装を行う。

アルゴリズム:
1. ファイル名を「数値トークン」と「文字列トークン」の列に分割する
2. トークン同士を比較する際、数値トークンは `u64` として比較、文字列トークンは大文字小文字を区別しない辞書順で比較する
3. ファイルシステムと同等の自然な順序を実現する

```rust
fn natural_sort_key(s: &str) -> Vec<NaturalToken> { ... }
enum NaturalToken { Num(u64), Str(String) }
```

---

## ドラッグ＆ドロップ

`eframe` の `dropped_files` 機能を使用する。

`update()` 内で毎フレーム:

```rust
let dropped = ctx.input(|i| i.raw.dropped_files.clone());
if let Some(file) = dropped.first() {
    if let Some(path) = &file.path {
        // ファイルがドロップされた場合
        if path.is_dir() {
            self.init_images(path, /* first file */ path, ctx);
        } else {
            self.init_images(&path.parent().unwrap(), path, ctx);
        }
    }
}
```

---

## UI レイアウト

```
┌─────────────────────────────────────────┐
│ [ウィンドウタイトル: filename[dir](WxH) N/Total]
├─────────────────────────────────────────┤
│                                         │
│           画像表示エリア                 │
│      (Contain モードで全体表示)          │
│                                         │
│  [キーオーバーレイ: 右下に半透明テキスト] │
└─────────────────────────────────────────┘
```

- 画像は `egui::Image::new(texture).fit_to_exact_size(available)` で画面に収まるよう表示
- 背景色はウィンドウのデフォルト（egui のデフォルトダーク/ライトテーマ）
- キーオーバーレイは `egui::Area` で画像の右下に重ねて表示し、`KEY_OVERLAY_SHOW_MS` 後に自動消去

---

## キーボードショートカット

`ctx.input(|i| ...)` で毎フレーム処理する。

| キー | 動作 |
|---|---|
| `A` / `←` / `H` | 前の画像 |
| `D` / `→` / `L` | 次の画像 |
| `↑` / `K` | 前の画像 |
| `↓` / `J` | 次の画像 |
| `W` / `PageUp` | 10 枚戻る |
| `S` / `PageDown` | 10 枚進む |
| `Shift+W` | 10 枚戻る |
| `Shift+S` | 10 枚進む |
| `Shift+A` / `Home` | 先頭画像 |
| `Shift+D` / `End` | 末尾画像 |
| `Space` | スライドショー ON/OFF |
| `Escape` | スライドショー停止 |
| `1` | 0° に回転 |
| `2` | 90° に回転 |
| `3` | 180° に回転 |
| `4` | 270° に回転 |
| `Q` | −90° 回転（相対） |
| `E` | +90° 回転（相対） |
| `Ctrl+↑` | 0° に回転 |
| `Ctrl+↓` | 180° に回転 |
| `Ctrl+←` | 270° に回転 |
| `Ctrl+→` | 90° に回転 |
| `Ctrl+S` | 現在の回転角で PNG として保存（PNG のみ） |
| `F` | ファイル名の表示/非表示 |
| `O` | ソート順の切り替え（自然順 / 更新日順） |
| `Ctrl+C` | ファイル本体をクリップボードにコピー |
| `Ctrl+Shift+C` | ファイルパスをコピー |
| `Ctrl+Shift+D` | フォルダパスをコピー |
| `Ctrl+O` | フォルダをファイルエクスプローラーで開く |
| `F5` / `Ctrl+R` | 画像をリロード |
| `Ctrl+F` | 画像のサイズに合わせてウインドウをリサイズ |
| `Ctrl+Q` | アプリケーションを終了 |

---

## 右クリックメニュー

`response.context_menu(...)` で表示する。各項目は `egui::Button::shortcut_text` で
対応するショートカットキーをラベル右側に表示し、選択された項目は `MenuAction` として
`apply_menu_action` に渡す。キーボードショートカットも同じ `apply_menu_action` を
経由するため、メニューとキー操作の挙動は常に一致する。

| 項目 | ショートカット |
|---|---|
| スライドショーを開始 / 停止 | `Space` |
| ファイル名を表示 / 非表示 | `F` |
| 更新日順 / 自然順にソート | `O` |
| ファイルをコピー | `Ctrl+C` |
| ファイルパスをコピー | `Ctrl+Shift+C` |
| フォルダパスをコピー | `Ctrl+Shift+D` |
| フォルダを開く | `Ctrl+O` |
| 画像を 0/90/180/270 度回転 | `1` / `2` / `3` / `4`（`Ctrl+↑→↓←`） |
| 画像をリロード | `F5` / `Ctrl+R` |
| 画像のサイズに合わせてウインドウをリサイズ | `Ctrl+F` |
| アプリケーションを終了 | `Ctrl+Q` |

---

## コーディング規約

- **変数名**: `snake_case`（Rust 標準）
- **定数**: `UPPER_SNAKE_CASE`
- **エラー処理**: `Result` を返す関数は `?` で伝播。UI 側では `unwrap_or_else` / `if let` を使い、エラー時は `show_key` でメッセージ表示
- **コメント**: 日本語・英語どちらも可
- **テクスチャ更新**: 必ず `egui::Context` を持つスコープ内で実施

---

## 設計上の注意（変更禁止事項）

1. **単一構造体設計**: 全ロジックが `App` 構造体に集約。別モジュール/トレイト分割は行わない
2. **`save_png` は PNG のみ**: 呼び出し元で拡張子チェックを行い、非 PNG ファイルには呼ばない
3. **拡張子マッチングは小文字比較**: `ext.to_ascii_lowercase()` で比較する
4. **自然順ソートは純 Rust 実装**: OS API には依存しない
5. **同期画像ロード**: `load_current_image` は `update()` 内で同期的に実行（大画像で一時的な UI 停止あり）
6. **コマンドライン引数は将来対応**: 当初はドラッグ＆ドロップのみ。コマンドライン引数は `std::env::args` で後から追加可能な設計とする

---

## ビルドコマンド

```bash
# デバッグビルド
cargo build

# リリースビルド
cargo build --release
# 出力: target/release/rview

# 実行
cargo run
```

---

## 動作確認項目

- [ ] ドラッグ＆ドロップ（画像ファイル / フォルダ）
- [ ] 前後ナビゲーション（キー・ループ）
- [ ] 10 枚ジャンプ・先頭/末尾ジャンプ
- [ ] スライドショー（自動遷移・停止）
- [ ] 回転（絶対・相対）
- [ ] 自然順ソート（`1, 2, 10, 11` 順）
- [ ] PNG 保存（回転反映）
- [ ] キーオーバーレイ表示・自動消去
- [ ] ウィンドウタイトル更新
