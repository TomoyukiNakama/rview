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
| `init_images(&mut self, dir: &Path, file: &Path, ctx: &egui::Context)` | `list_up_images` → `natural_sort` → `load_current_image`。あわせて `watch_dir` でフォルダ監視を開始 |
| `watch_dir(&mut self, dir: &Path, ctx: &egui::Context)` | `notify` クレートで `dir` を非再帰監視する（監視先が変わったときだけ作り直す） |
| `poll_fs_events(&mut self, ctx: &egui::Context)` | 監視イベントを取り込み、最後のイベントから `FS_REFRESH_DEBOUNCE_MS`（300ms）静かになったら `refresh_image_list` を呼ぶ |
| `refresh_image_list(&mut self, changed, ctx)` | 表示中の画像を保ったまま一覧を取り直す。表示中の画像が消えていれば同じ位置の画像を表示し、内容が変わった画像のキャッシュは破棄する |
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

### 画像サイズの変更

右クリックメニューの「画像サイズの変更...」で `egui::Modal` のダイアログを開く（`open_resize_dialog` / `show_resize_dialog`）。

- 入力欄: 幅・高さ（1–65535 px、画面に表示している回転後の向き）、「アスペクト比を固定」チェックボックス（既定でオン。幅・高さの一方を変えるともう一方を元の比率で合わせる）
- [リサイズ]: 指定サイズを `pending_resize`（対象ファイルと、元画像の向きでの幅・高さ）に保持して閉じる。元のサイズと同じなら解除する
- [リセット]: 入力欄を元のサイズに戻す
- [キャンセル] / `Esc` / ダイアログ外クリック: 何もせず閉じる

サイズ変更はその場ではファイルに書き込まず、回転・反転と同じく `Ctrl+S`（上書き）/ `Ctrl+Shift+S`（新規、`元名_800x600.ext` など）で保存する。
保存時は元画像の向きで拡大縮小（Lanczos3、GIF はパレットのため最近傍法）してから回転・反転を施し、EXIF の PixelX/YDimension を新しい大きさに更新する。
保留中のサイズは対象の画像にだけ適用し、他の画像には引き継がない。ダイアログ表示中はキーボードショートカットを無効にする。

### フィルター

右クリックメニューの「フィルターを表示 / 閉じる」で、ウインドウ左上（リサイズウインドウと同じ高さ）を初期位置とし、ドラッグで移動できる非モーダルの `egui::Window` を開閉する（`show_filter_window`、条件の判定は `src/filter.rs`）。
入力が変わるたびにすぐ絞り込む。ウインドウを閉じても条件は有効のままで、解除は [クリア] で行う。

- ファイル名: 大文字小文字を区別しない部分一致。空白で区切ると AND、`*`（0 文字以上）/ `?`（1 文字）を含む語はファイル名全体とのワイルドカード一致
- 幅・高さ: 下限〜上限（px、両端を含む）。空欄は制限なし。数値として読めない欄は無視して警告を出す
- 一致件数（`一致: 12 / 340 件`）を表示する。絞り込み中はウインドウタイトルに ` [-Filter: 総数]` を付ける

一覧は `all_images`（フォルダ内の全画像）と `images`（フィルター適用後の表示対象）の 2 つを持つ。`list_up_images` は両方を作り、
`apply_filter` は条件変更時に `images` だけを作り直す（表示中の画像が条件に合えばそのまま、外れたら先頭を表示）。
解像度は解像度の条件があるときだけ、`image::image_dimensions`（ヘッダのみ読む）でバックグラウンドに取得して `dim_cache` に溜める（`poll_dimensions`）。
取得中は `FILTER_REAPPLY_MS`（200ms）ごとに絞り込みを反映し、未取得の画像は条件に合わないものとして扱う。フォルダ監視で内容が変わった画像は取り直す。

### リサイズウインドウ（連続リサイズ）

右クリックメニューの「リサイズウインドウを表示 / 閉じる」で、ウインドウ右上を初期位置とし、ドラッグで移動できる非モーダルの `egui::Window` を開閉する（`show_resize_window`）。
画像を切り替えても、ウインドウを閉じても入力値（`resize_window`）は保持する。初めて開いたときは表示中の画像のサイズで初期化する。

- 入力欄: 幅・高さ（1–65535 px、表示の向き）、「アスペクト比を固定」チェックボックス
- アスペクト比を固定: オンのとき、幅・高さの一方を変えると表示中の画像の比率でもう一方を合わせる。保存時は各画像の縦横比を保ったまま 幅 x 高さ の枠に収まる大きさにする（実際の保存サイズをウインドウに表示）。オフなら指定どおりの 幅 x 高さ にする
- [リサイズして別名保存]: 表示中の画像を、回転・反転も施して `元名_800x600.ext` などの別名で保存し、保存したファイルに表示を切り替える（`save_resized_as_new`）。保存はバックグラウンドスレッドで行い（`SaveJob` / `poll_save_job`）、保存中は元の画像を表示したまま「保存中...」を出す。保存完了後に保存したファイルをデコードし、読み込みが終わってから表示を切り替える（`finish_switch_to_saved`）。保存中に別の画像へ移った場合は切り替えない。
  保存中でも続けて受け付け、`save_queue` に積んで受け付け順に 1 件ずつ保存する（オーバーレイに「保存中... (残り N 件)」）。サイズ・回転・反転・保存先は受け付けた時点で決め、保存先は実行中・実行待ちのものとも重ならない名前にする（`unique_path_excluding`）。表示の切り替えは、終わった保存の元画像をそのとき表示している場合だけ行う。画像ダイアログで指定した保留中のサイズは使わない
- [閉じる]: ウインドウを閉じる
- ウインドウの表示中は `Ctrl+S` が上書き保存ではなく [リサイズして別名保存] になる（入力欄の編集中も有効）

入力欄の編集中（`ctx.wants_keyboard_input()`）はキーボードショートカットを無効にする。編集していないときは、ウインドウを開いたままいつもどおり画像を切り替えられる。

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

### 他のアプリへのファイルドラッグ（Linux）

表示中の画像をドラッグすると、ファイル（`text/uri-list`）を他のアプリへ渡す。

- 既定はコピー、`Ctrl` を押しながらドラッグすると移動。受け手やコンポジターが勝手に移動を選ばないよう、許可するアクションはどちらか一方だけにする。移動の場合もファイルの移動は受け手（ファイルマネージャー）が行い、rview は元ファイルを消さない
  - Wayland: `wl_data_source.set_actions` は `start_drag` の前に一度しか呼べないため、ドラッグ開始時の `Ctrl` で決まる
  - X11: `XdndPosition` のアクションを毎回送るので、ドラッグ中に `Ctrl` を押し直すと切り替わる
- ドラッグアイコン（`src/drag_icon.rs`）: サムネイル（長辺 128 論理 px、少し透過）、その下にファイル名（長ければ … で省略）、サムネイル左上に「＋コピー」（緑）/「→移動」（オレンジ）のバッジ
  - サムネイルはデコードスレッドでテクスチャと一緒に作り `thumbnail_cache` に持つ（ドラッグ開始時に読み直さない）
  - Wayland: 共有メモリのバッファを作ってドラッグアイコン用サーフェスに貼る（`create_image_icon_surface`、整数倍率で `set_buffer_scale`）。サーフェスの `commit` は `start_drag` の後に行う（ドラッグアイコンの役割が付く前に commit した内容は mutter が表示しない）
  - X11: ポインタの右下に追従する override-redirect ウインドウで表示する（24 ビット画面のみ。透過できないので暗い背景に合成）

### rview へのドロップ

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
| 画像サイズの変更... | なし（ダイアログを開く） |
| リサイズウインドウを表示 / 閉じる | なし |
| フィルターを表示 / 閉じる | なし |
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
