# view — WPF Image Viewer

これは移植元のイメージビューワーの仕様です。
これをもとにRustのイメージビューワーを作成します。

## プロジェクト概要

`view` は WPF (Windows Presentation Foundation) 製の Windows 専用デスクトップイメージビューア。
ドラッグ＆ドロップで画像ファイル/フォルダを開き、キーボード・マウスホイール・タッチジェスチャーで操作できる。

- **ビルド**: Linux から `dotnet build` で可能
- **実行**: WPF のため **Windows のみ**（Linux では実行不可）

---

## Tech Stack

| 項目 | 値 |
|---|---|
| 言語 | C# |
| フレームワーク | .NET 10.0-windows |
| UI | WPF (Windows Presentation Foundation) |
| 出力タイプ | WinExe（コンソールなし） |
| NuGet パッケージ | なし（WPF 組み込みコーデックのみ） |
| Nullable | 有効 |
| ImplicitUsings | 有効 |

---

## 移植元リポジトリの構造

移植元の C# ソースは本リポジトリには含めていない（別プロジェクトのため）。
以下は移植時点での移植元リポジトリ `view` の構成。

```
view/                              ← リポジトリルート
├── view.slnx                      ← ソリューションファイル
└── view/                          ← C# プロジェクト（同名サブディレクトリに注意）
    ├── view.csproj
    ├── App.xaml / App.xaml.cs     ← WPF アプリエントリポイント
    ├── MainWindow.xaml            ← XAML UI 定義（124 行）
    ├── MainWindow.xaml.cs         ← 全アプリケーションロジック（1,852 行）
    ├── AssemblyInfo.cs
    └── Properties/PublishProfiles/
        └── ClickOnceProfile.pubxml
```

ビルド成果物（`.gitignore` 候補）: `view/bin/`, `view/obj/`

---

## ビルドコマンド

```bash
# デバッグビルド（Linux から実行可能）
dotnet build view.slnx

# リリースビルド
dotnet build view/view.csproj -c Release
# 出力: view/bin/Release/net10.0-windows/view.exe
```

テストプロジェクトは存在しない。動作確認は Windows マシンで手動実施。

---

## アーキテクチャ概要

**単一クラス設計**: `MainWindow` クラスに全ロジックを集約。MVVM なし、ViewModel なし、外部 DI なし。
この設計は意図的なもので、MVVM へのリファクタリングは原則行わない。

### 主要フィールド（`MainWindow`）

| フィールド | 型 | 用途 |
|---|---|---|
| `CurrentDir` | `DirectoryInfo` | 作業ディレクトリ（初期値 `./`） |
| `CurrentFile` | `FileInfo` | 現在表示中のファイル |
| `CurrentImage` | `BitmapImage` | 現在ロード済みのビットマップ |
| `Images` | `List<FileInfo>` | 現在ディレクトリの画像ファイル一覧 |
| `CurrentIndex` | `int` | `Images` 内の現在インデックス |
| `IsSliding` | `bool` | スライドショー稼働中フラグ |
| `IsDroped` | `bool` | ドロップ処理済みフラグ |
| `manipulationScale` | `double` | ピンチズーム倍率 |
| `ImagePattern` | `string` | 対応拡張子: `.jpeg,.jpg,.bmp,.gif,.tiff,.png,.webp` |

### DispatcherTimer 一覧

| フィールド | インターバル | 用途 |
|---|---|---|
| `SlideShowTimer` | 3000 ms (`SLIDE_SHOW_INTERVAL`) | 次の画像へ自動遷移 |
| `scrollTimer` | 200 ms (`SCROLL_WATCH_INTERVAL`) | ホイール非操作後に `isShiftable` をリセット |
| `KeyMonitorTimer` | 200 ms（1000 ms 後に非表示: `KEY_MONITOR_SHOW_INTERVAL`） | キーオーバーレイの表示制御 |

### 画像変換

- **回転**: `MainImage.RenderTransform = new RotateTransform(angle)`
- **ピンチズーム**: `MainImage.LayoutTransform = new ScaleTransform(scale, scale)`
- 2 つの変換は異なるプロパティに適用されるため干渉しない

---

## 主要クラス・メソッドリファレンス

### `MainWindow`（[view/MainWindow.xaml.cs](view/MainWindow.xaml.cs)）

#### 初期化

| メソッド | 役割 |
|---|---|
| `MainWindow()` | タイマー・ジェスチャーハンドラ・Stretch 設定を初期化 |
| `SetupTimer()` | `SlideShowTimer` を作成（未開始） |
| `SetupKeyMonitor()` | `KeyMonitorTimer` を作成・開始 |

#### 画像ロード・ナビゲーション

| メソッド | 役割 |
|---|---|
| `ListUpImages(string DirPath)` | ディレクトリから対応拡張子ファイルを列挙して `Images` に格納 |
| `InitImages(string DirPath, string FilePath)` | `ListUpImages` → `NaturalSort` → `LoadCurrentImage` |
| `LoadCurrentImage()` | `CurrentFile` を `RefreshMainImage` で表示、`txtFileName` 更新 |
| `RefreshMainImage(FileInfo, bool)` | `FileStream` + `BitmapCacheOption.OnLoad` + `Freeze()` でビットマップロード |
| `ShiftImage(bool IsNext)` | 前後ナビゲーション（端でループ） |
| `LoadFirstImage()` | インデックス 0 へジャンプ |
| `LoadLastImage()` | 最終インデックスへジャンプ |
| `LoadPgUpImage()` | 10 枚戻る（0 でクランプ） |
| `LoadPgDnImage()` | 10 枚進む（最大でクランプ） |
| `SetCurrentIndex(FileInfo)` | ファイル名で `Images` を検索し `CurrentIndex` を設定 |

#### スライドショー

| メソッド | 役割 |
|---|---|
| `SlideShow(bool Active)` | `SlideShowTimer` の開始/停止、`IsSliding` とタイトル更新 |

#### 回転

| メソッド | 役割 |
|---|---|
| `RotateMainImage(double Angle)` | 絶対角度で回転（`RenderTransform`） |
| `RotateMainImageAdd(double DeltaAngle)` | 現在角度に加算（0–359 範囲に正規化） |
| `GetRotationAngle(Image)` | `LayoutTransform` / `RenderTransform` / `TransformGroup` から現在角度を取得 |
| `SetBackgroundByAngle(double)` | 回転角に合わせてグラデーション方向を変更 |

#### ファイル操作

| メソッド | 役割 |
|---|---|
| `SavePNG(double Angle)` | PNG ファイルのみ回転保存（`PngBitmapDecoder` + `TransformedBitmap` + `PngBitmapEncoder`） |
| `RenameFile(FileInfo, string)` | ファイルリネーム後 `Images` リストを更新 |

#### ユーティリティ

| メソッド | 役割 |
|---|---|
| `SetTitle()` | ウィンドウタイトルを `filename[dir](WxH) N/Total [-Slide] [-Rotated: X°]` 形式で設定 |
| `NaturalSort()` | `NaturalStringComparer` で自然順ソート |
| `ShowExceptionMessage(string)` | `Task.Run` + `Dispatcher.Invoke` で MessageBox 表示（UI スレッドブロック回避） |
| `ShowKey(string)` | キーオーバーレイ (`txtKey`) に操作内容を表示 |

### `NaturalStringComparer`（`MainWindow` 内ネストクラス）

`IComparer<string>` を実装。`shlwapi.dll` の `StrCmpLogicalW`（P/Invoke）を使用。
Windows Explorer と同じ順序（数値部分を数値として比較）。スタティック singleton `Windows` プロパティで取得。

---

## キーボードショートカット

| キー | 動作 |
|---|---|
| A / D / 左右矢印 / H / L | 前の画像 / 次の画像 |
| 上下矢印 / J / K | 次の画像 / 前の画像 |
| W / PageUp | 10 枚戻る |
| S / PageDown | 10 枚進む |
| Shift+W | 10 枚戻る |
| Shift+S | 10 枚進む |
| Shift+A / Home | 先頭画像 |
| Shift+D / End | 末尾画像 |
| Space | スライドショー ON/OFF |
| Escape | スライドショー停止 |
| 1 | 0° に回転 |
| 2 | 90° に回転 |
| 3 | 180° に回転 |
| 4 | 270° に回転 |
| Q | −90° 回転（相対） |
| E | +90° 回転（相対） |
| Ctrl+↑ | 0° に回転 |
| Ctrl+↓ | 180° に回転 |
| Ctrl+← | 270° に回転 |
| Ctrl+→ | 90° に回転 |
| Ctrl+S | 現在の回転角で PNG として保存 |

---

## コーディング規約

- **パラメータ名**: PascalCase（例: `bool IsNext`, `double Angle`）— 既存スタイルを維持すること
- **定数**: UPPER_SNAKE_CASE（例: `SLIDE_SHOW_INTERVAL`, `SWIPE_THRESHOLD`）
- **イベントハンドラ**: `try { } catch (Exception ex) { ShowExceptionMessage(ex.Message); }` で包む
- **コメント**: 日本語・英語どちらも可
- **UI 更新**: 必ず Dispatcher スレッドで実施。`RefreshMainImage` では `bitmap.Freeze()` を呼ぶ

---

## 設計上の注意（変更禁止事項）

1. **単一クラス設計**: 全ロジックが `MainWindow.xaml.cs` に集約。MVVM へのリファクタは行わない
2. **`SavePNG` は PNG のみ**: `PngBitmapDecoder` を使用するため非 PNG ファイルは例外発生。ハンドラ側でガード済み
3. **拡張子マッチングは substring ベース**: `ext.Contains(p)` による実装は意図的なもの、変更しない
4. **`NaturalStringComparer` は Windows API 依存**: `StrCmpLogicalW` を純粋 managed 実装に置き換えない
5. **同期画像ロード**: `RefreshMainImage` は UI スレッドで同期的に実行（大画像で一時的な UI 停止あり）
6. **コマンドライン引数なし**: ファイルを開く手段はドラッグ＆ドロップのみ

---

## 開発ワークフロー

1. `view/view/` 内のファイル（主に `MainWindow.xaml` / `MainWindow.xaml.cs`）を編集
2. `dotnet build` でコンパイルエラーを確認（Linux から実行可能）
3. Windows マシン/VM に転送して手動動作確認
4. 確認項目: ドラッグ＆ドロップ、ナビゲーション、スライドショー、回転・保存

