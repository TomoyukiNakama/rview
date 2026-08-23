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
起動時に以下の順で日本語フォントを探し、見つからなければ日本語が表示できない（豆腐になる）。
対応形式は `.ttf` / `.otf` / `.ttc` / `.otc`。

1. 環境変数 `RVIEW_FONT` による明示指定

   ```bash
   RVIEW_FONT=/path/to/font.ttf rview
   # .ttc（フォントコレクション）はフェイス番号を付けて指定できる
   RVIEW_FONT=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc#0 rview
   ```

2. 同梱フォント（実行ファイルのあるディレクトリとその上位、またはカレントディレクトリからの相対パス）

   ```
   assets/fonts/HackGen_NF_v2.10.0/HackGenConsoleNF-Regular.ttf
   ```

   開発時に使用しているフォントは [HackGen (白源)](https://github.com/yuru7/HackGen)（SIL Open Font License 1.1）。
   [Noto Sans JP](https://fonts.google.com/noto/specimen/Noto+Sans+JP) など他の `.ttf` でもよい。

3. fontconfig（`fc-match :lang=ja`）が返す日本語フォント

4. フォントディレクトリ（`~/.fonts`、`~/.local/share/fonts`、`/usr/share/fonts`、
   macOS の `Library/Fonts`、Windows の `%WINDIR%\Fonts` など）の走査

いずれの候補も実際に読み込んで「あ」「漢」のグリフを持つことを確認してから採用するため、
日本語を含まないフォントや壊れたファイルを掴むことはない。

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
