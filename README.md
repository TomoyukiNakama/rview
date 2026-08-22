# rview

Rust + [egui](https://github.com/emilk/egui) 製のクロスプラットフォーム画像ビューワー。
WPF 製の Windows 専用ビューワー `view` を移植したもの。

- ドラッグ＆ドロップで画像ファイル／フォルダを開く
- キーボードとマウスホイールで前後の画像へ移動
- スライドショー、回転、回転を適用した PNG 保存
- ファイル本体／パスのクリップボードコピー、ファイルドラッグによる他アプリへの受け渡し

対応形式: JPEG / PNG / BMP / GIF / TIFF / WebP

## ビルド

```bash
cargo build --release
# 出力: target/release/rview
```

Linux では X11 / Wayland の両方で動作する。

## 実行

```bash
rview                    # カレントディレクトリの画像を開く
rview path/to/image.jpg  # 指定した画像を開く
rview path/to/dir        # 指定したディレクトリの画像を開く
```

## 日本語表示用フォントの設定

フォントファイルはサイズが大きいためリポジトリに含めていない。
未設定でも起動するが、日本語のファイル名などが表示できない（豆腐になる）。

次のいずれかの方法でフォントを用意する。egui の制約により、
**`.ttf` / `.otf` のみ対応**（`.ttc` フォントコレクションは読み込めない）。

1. 環境変数で指定する

   ```bash
   RVIEW_FONT=/path/to/font.ttf rview
   ```

2. 既定のパスに配置する（実行ファイルと同じディレクトリ、またはカレントディレクトリからの相対パス）

   ```
   assets/fonts/HackGen_NF_v2.10.0/HackGenConsoleNF-Regular.ttf
   ```

   開発時に使用しているフォントは [HackGen (白源)](https://github.com/yuru7/HackGen)（SIL Open Font License 1.1）。
   [Noto Sans JP](https://fonts.google.com/noto/specimen/Noto+Sans+JP) など他の `.ttf` でもよい。

3. Linux では以下のシステムフォントが存在すれば自動で使われる

   ```
   /usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf
   /usr/share/fonts/truetype/fonts-japanese-gothic.ttf
   /usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf
   /usr/share/fonts/truetype/ipafont/ipagp.ttf
   /usr/share/fonts/truetype/ipafont-gothic/ipagp.ttf
   ```

## 主なキーボードショートカット

| キー | 動作 |
|---|---|
| `A` / `←` / `H` / `↑` / `K` | 前の画像 |
| `D` / `→` / `L` / `↓` / `J` | 次の画像 |
| `W` / `PageUp` | 10 枚戻る |
| `S` / `PageDown` | 10 枚進む |
| `Shift+A` / `Home` | 先頭の画像 |
| `Shift+D` / `End` | 末尾の画像 |
| `Space` | スライドショー ON/OFF |
| `1` / `2` / `3` / `4` | 0° / 90° / 180° / 270° に回転 |
| `Q` / `E` | −90° / +90° 回転 |
| `Ctrl+S` | 現在の回転角で PNG として保存 |
| `F` | ファイル名の表示/非表示 |
| `O` | ソート順の切り替え（自然順 / 更新日順） |
| `Ctrl+C` | ファイル本体をコピー |
| `Ctrl+Shift+C` | ファイルパスをコピー |
| `Ctrl+F` | 画像サイズに合わせてウィンドウをリサイズ |
| `Ctrl+Q` | 終了 |

全ショートカットと詳細仕様は [spec.md](spec.md) を参照。右クリックメニューからも同じ操作が行える。

## ドキュメント

- [spec.md](spec.md) — rview の仕様
- [doc/port.md](doc/port.md) — 移植元 WPF アプリの仕様

## ライセンス

MIT License — 詳細は [LICENSE](LICENSE) を参照。

フォントは本リポジトリに含まれていないため、MIT の対象外。
利用するフォントのライセンス（HackGen は SIL OFL 1.1）は各配布元の条件に従うこと。
