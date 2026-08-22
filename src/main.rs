// 必要なクレートやモジュールをインポートします。
use eframe::egui; // GUIフレームワーク eframe と egui
use egui::{ColorImage, TextureHandle, TextureOptions}; // eguiから画像関連の型
use std::path::{Path, PathBuf}; // ファイルパスを扱うための型
use std::time::{Duration, Instant}; // 時間を扱うための型

// ソート順を定義する列挙型
#[derive(PartialEq, Clone, Copy)]
enum SortOrder {
    Natural,      // 自然順（ファイル名の数値を考慮した辞書順）
    ModifiedDate, // 更新日時の新しい順
}

// デスクトップ環境の種別を定義する列挙型
// 検出は Linux でのみ行うが、クリップボード処理が全プラットフォームで
// この型を引数に取るため、型自体は cfg で切り分けない
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum DisplayServer {
    X11,
    Wayland,
}

// 右クリックメニューのアクションを定義する列挙型
enum MenuAction {
    ToggleSlideShow,    // スライドショーの開始/停止
    ToggleFilename,     // ファイル名の表示/非表示
    ToggleSortOrder,    // ソート順の切り替え
    CopyFile,           // ファイル本体をクリップボードにコピー（ファイラーに貼り付け可能）
    CopyFilePath,       // ファイルパスをクリップボードにコピー
    CopyFolderPath,     // 親フォルダのパスをクリップボードにコピー
    OpenFolder,         // 親フォルダをファイルエクスプローラーで開く
    Rotate(f32),        // 指定した角度で画像を回転
    Reload,             // 現在の画像を再読み込み
    ResizeWindow,       // 画像のサイズに合わせてウインドウをリサイズ
    Quit,               // アプリケーションを終了
}

// 定数定義
const SLIDE_SHOW_INTERVAL_MS: u64 = 3000; // スライドショーの画像切り替え間隔 (ミリ秒)
const KEY_OVERLAY_SHOW_MS: u64 = 1000;    // キー操作のオーバーレイ表示時間 (ミリ秒)
const PAGE_JUMP_COUNT: usize = 10;        // PageUp/PageDownでの移動枚数
const SCROLL_DEBOUNCE_MS: u64 = 50;     // スクロールによる画像切り替えのデバウンス間隔 (ミリ秒)
const SCROLL_THRESHOLD: f32 = 10.0;        // スクロール検出の閾値 (ポイント)
// スクロール入力がこの時間途切れたら蓄積量をリセットする。
// リセットしないと、閾値未満の微小スクロールが時間をまたいで累積し、
// 意図しないタイミングで画像が切り替わる。
const SCROLL_RESET_MS: u64 = 150;
const SUPPORTED_EXTENSIONS: &[&str] = &[  // サポートする画像ファイルの拡張子
    "jpeg", "jpg", "bmp", "gif", "tiff", "tif", "png", "webp",
];
// 現在の画像の前後にキャッシュする枚数。
// デコード済み画像は非圧縮 RGBA で保持されるため（4000x3000 で約 48MB/枚）、
// 大きすぎるとメモリと VRAM を食い潰す。前後 8 枚 = 最大 17 枚を上限とする。
const CACHE_SIZE: usize = 8;
// 同時に走らせるデコードスレッドの上限。
// 一度に全キャッシュ対象を spawn するとメモリと CPU が飽和し、
// 表示すべき画像のデコードが後回しになるため制限する。
const MAX_CONCURRENT_DECODES: usize = 3;
// ファイルドラッグ開始とみなすポインタ移動量 (ポイント)。ドラッグ送出は Linux / macOS のみ
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
const DRAG_START_THRESHOLD: f32 = 6.0;

// main関数: アプリケーションのエントリーポイント5
fn main() -> eframe::Result<()> {
    // ネイティブアプリケーションとして実行
    // Wayland (特に KDE Plasma) では初期ウィンドウサイズと app_id を明示しないと
    // サーフェスが正しくマップされず、ウィンドウが初回表示されない問題があるため設定する
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("rview")                 // Wayland の app_id (KDE のウィンドウ統合に必要)
            .with_inner_size([1280.0, 720.0])      // 初期ウィンドウサイズ
            .with_min_inner_size([320.0, 240.0])   // 最小サイズ
            .with_visible(true)
            .with_active(true),
        ..Default::default()
    };
    eframe::run_native(
        "rview", // ウィンドウのタイトル
        native_options,
        Box::new(|cc| {
            // eguiコンテキストが作成されたときに呼ばれるクロージャ
            setup_custom_fonts(&cc.egui_ctx); // カスタムフォントをセットアップ
            let mut app = App::new(); // App構造体の新しいインスタンスを生成
            app.init_from_args(&cc.egui_ctx); // コマンドライン引数で渡されたファイル/ディレクトリを開く
            Ok(Box::new(app))
        }),
    )
}

// アプリケーションの状態を保持する構造体
struct App {
    current_dir: PathBuf,                     // 現在表示している画像があるディレクトリ
    current_file: Option<PathBuf>,            // 現在表示している画像のファイルパス
    current_texture: Option<TextureHandle>,   // 現在表示している画像のテクスチャハンドル
    current_image_size: Option<(u32, u32)>,   // 現在の画像の元のサイズ (幅, 高さ)
    images: Vec<PathBuf>,                     // ディレクトリ内の画像ファイル一覧
    current_index: usize,                     // imagesベクタ内での現在の画像のインデックス
    is_sliding: bool,                         // スライドショーが有効かどうか
    slide_last: Instant,                      // 最後にスライドショーで画像を変更した時刻
    rotation_angle: f32,                      // 画像の回転角度 (度)
    key_overlay_text: String,                 // オーバーレイ表示するテキスト
    key_overlay_hide_at: Option<Instant>,     // オーバーレイを非表示にする時刻
    last_scroll_shift: Option<Instant>,        // スクロールデバウンス用タイムスタンプ
    last_scroll_input: Option<Instant>,        // 最後にホイール入力があった時刻（蓄積リセット判定用）
    scroll_accumulator: f32,                  // マウスホイールのスクロール量の蓄積
    show_filename: bool,                      // ファイル名を左下に表示するかどうか
    last_title: String,                       // 最後に送信したウィンドウタイトル（変化時のみ送るため）
    texture_cache: std::collections::HashMap<PathBuf, TextureHandle>, // テクスチャキャッシュ
    loading_images: std::collections::HashSet<PathBuf>, // 読み込み中の画像セット
    texture_sender: std::sync::mpsc::Sender<DecodedImage>,
    texture_receiver: std::sync::mpsc::Receiver<DecodedImage>,
    cache_generation: u64,                    // 画像リスト／ファイル内容が変わるたびに増える
    display_server: Option<DisplayServer>,  // 検出されたデスクトップ環境（検出は Linux のみ）
    #[cfg(target_os = "linux")]
    x11_window_id: Option<u32>,  // X11 ウィンドウIDのキャッシュ（XDND用）
    #[cfg(target_os = "linux")]
    wayland_dnd: Option<WaylandDndContext>, // Wayland DnD コンテキスト
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pending_file_drag: bool,      // ドラッグを開始するフラグ
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    file_drag_triggered: bool,    // 現在のドラッグジェスチャで既に DnD を起動したか
    sort_order: SortOrder,        // 画像リストのソート順
}

/// バックグラウンドスレッドから返るデコード結果。
///
/// `generation` は投入時点の `App::cache_generation`。受信時に現在値と食い違って
/// いればその結果は破棄する。これがないと、別ディレクトリを開いた後や、回転画像を
/// 上書き保存した後に、古いファイル内容のデコード結果が遅れて到着して
/// キャッシュと表示を巻き戻してしまう。
struct DecodedImage {
    path: PathBuf,
    generation: u64,
    result: Result<(ColorImage, (u32, u32)), String>,
}

// App構造体の実装
impl App {
    // 新しいAppインスタンスを作成する
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            current_dir: PathBuf::from("./"), // 実行ディレクトリを初期値に
            current_file: None,
            current_texture: None,
            current_image_size: None,
            images: Vec::new(),
            current_index: 0,
            is_sliding: false,
            slide_last: Instant::now(),
            rotation_angle: 0.0,
            key_overlay_text: String::new(),
            key_overlay_hide_at: None,
            last_scroll_shift: None,
            last_scroll_input: None,
            scroll_accumulator: 0.0,
            show_filename: true,
            last_title: String::new(),
            texture_cache: std::collections::HashMap::new(),
            loading_images: std::collections::HashSet::new(),
            texture_sender: tx,
            texture_receiver: rx,
            cache_generation: 0,
            display_server: None,
            #[cfg(target_os = "linux")]
            x11_window_id: None,
            #[cfg(target_os = "linux")]
            wayland_dnd: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            pending_file_drag: false,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            file_drag_triggered: false,
            sort_order: SortOrder::Natural,
        }
    }

    // 指定されたディレクトリ内の画像ファイルをリストアップする
    fn list_up_images(&mut self, dir: &Path) {
        self.images.clear(); // 既存のリストをクリア
        let Ok(entries) = std::fs::read_dir(dir) else {
            // ディレクトリが読めなければ何もしない
            return;
        };
        // ディレクトリエントリをフィルタリングして画像ファイルのみを収集
        self.images = entries
            .filter_map(|e| e.ok()) // ResultをOptionに変換し、エラーを除外
            .map(|e| e.path())       // DirEntryをPathBufに変換
            .filter(|p| {
                // ファイルであり、かつサポートされている拡張子を持つかチェック
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| {
                            let lower = e.to_ascii_lowercase(); // 拡張子を小文字に
                            SUPPORTED_EXTENSIONS.contains(&lower.as_str()) // サポートリストに含まれるか
                        })
                        .unwrap_or(false) // 拡張子がない場合はfalse
            })
            .collect(); // フィルタリングされたパスをVecに収集
    }

    // 画像リストを自然順ソートする
    fn natural_sort(&mut self) {
        self.images.sort_by(|a, b| {
            let an = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let bn = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
            natural_cmp(an, bn) // 自然順比較関数を呼ぶ
        });
    }

    // 画像リストを更新日時の新しい順にソートする
    fn sort_by_modified(&mut self) {
        self.images.sort_by(|a, b| {
            let ta = a.metadata().and_then(|m| m.modified()).ok();
            let tb = b.metadata().and_then(|m| m.modified()).ok();
            tb.cmp(&ta) // 新しい順（降順）
        });
    }

    // 現在のソート順を適用する
    fn apply_sort(&mut self) {
        match self.sort_order {
            SortOrder::Natural => self.natural_sort(),
            SortOrder::ModifiedDate => self.sort_by_modified(),
        }
    }

    // ファイルパスから現在のインデックスを設定する
    fn set_current_index_by_path(&mut self, path: &Path) {
        self.current_index = self
            .images
            .iter()
            .position(|p| p == path) // パスが一致する最初の要素のインデックスを探す
            .unwrap_or(0); // 見つからなければ0
    }

    // 画像リストを初期化し、指定された画像または最初の画像を表示する
    fn init_images(&mut self, dir: &Path, file: Option<&Path>, ctx: &egui::Context) {
        // キャッシュをリセットする。
        // loading_images は実際に走っているスレッドの台帳なのでクリアしない。
        // クリアすると同時実行数の計算が狂い、同じ画像を二重にデコードしてしまう。
        // 代わりに世代を進めて、走行中のデコード結果を受信時に破棄させる。
        self.texture_cache.clear();
        self.cache_generation = self.cache_generation.wrapping_add(1);

        self.list_up_images(dir); // 画像をリストアップ
        if self.images.is_empty() {
            // 画像がなければテクスチャなどをクリアして終了
            self.current_texture = None;
            self.current_file = None;
            self.current_image_size = None;
            return;
        }
        self.apply_sort(); // 現在のソート順で並べ替え
        match file {
            // 表示するファイルが指定されていればそのインデックスを、なければ先頭のインデックスを設定
            Some(f) => self.set_current_index_by_path(f),
            None => self.current_index = 0,
        }
        self.current_dir = dir.to_path_buf(); // 現在のディレクトリを更新
        self.load_current_image(ctx); // 現在のインデックスの画像を読み込む
    }

    // 起動時のコマンドライン引数を処理する
    // ファイルが渡された場合はその親ディレクトリを開いて当該ファイルを表示し、
    // ディレクトリが渡された場合はそのディレクトリを開く。
    fn init_from_args(&mut self, ctx: &egui::Context) {
        let Some(arg) = std::env::args().nth(1) else {
            return; // 引数がなければ何もしない
        };
        // 相対パスでも list_up_images が返す絶対パスと一致するよう正規化する
        let path = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
        if path.is_dir() {
            // ディレクトリが渡された場合はそのまま開く
            self.init_images(&path, None, ctx);
        } else if path.is_file() {
            // ファイルが渡された場合は親ディレクトリを開いてそのファイルを表示する
            let dir = path.parent().unwrap_or(&path).to_path_buf();
            self.init_images(&dir, Some(&path), ctx);
        }
    }

    // 現在のインデックスにある画像をバックグラウンドスレッドで読み込む
    fn load_current_image(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return; // 画像リストが空なら何もしない
        }
        let path = self.images[self.current_index].clone();
        self.current_file = Some(path.clone());
        self.rotation_angle = 0.0; // 読み込み時に回転をリセット

        // まずキャッシュからテクスチャを探す
        if let Some(texture) = self.texture_cache.get(&path) {
            self.current_texture = Some(texture.clone());
            // キャッシュされたテクスチャからサイズを取得
            let size = texture.size();
            self.current_image_size = Some((size[0] as u32, size[1] as u32));
        } else {
            // キャッシュにない場合は、現在のテクスチャを一旦クリアして読み込み中を示す
            self.current_texture = None;
            self.current_image_size = None;
        }

        // 現在位置に基づいてキャッシュを更新し、必要な画像をプリフェッチする
        self.update_cache(ctx);
    }

    // 画像を次に進めるか前に戻す
    fn shift_image(&mut self, is_next: bool, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }
        if is_next {
            // 次の画像へ (末尾の次は先頭)
            self.current_index = (self.current_index + 1) % self.images.len();
        } else {
            // 前の画像へ (先頭の前は末尾)
            self.current_index = if self.current_index == 0 {
                self.images.len() - 1
            } else {
                self.current_index - 1
            };
        }
        self.load_current_image(ctx); // 新しいインデックスの画像を読み込む
    }

    // 最初の画像を読み込む
    fn load_first_image(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }
        self.current_index = 0;
        self.load_current_image(ctx);
    }

    // 最後の画像を読み込む
    fn load_last_image(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }
        self.current_index = self.images.len() - 1;
        self.load_current_image(ctx);
    }

    // PAGE_JUMP_COUNT分だけ前に画像をジャンプする
    fn load_pgup_image(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }
        self.current_index = self.current_index.saturating_sub(PAGE_JUMP_COUNT); // 0未満にならないように
        self.load_current_image(ctx);
    }

    // PAGE_JUMP_COUNT分だけ次に画像をジャンプする
    fn load_pgdn_image(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }
        self.current_index =
            (self.current_index + PAGE_JUMP_COUNT).min(self.images.len() - 1); // 配列の範囲を超えないように
        self.load_current_image(ctx);
    }

    // キャッシュを更新し、必要な画像を非同期でプリフェッチする
    fn update_cache(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() {
            return;
        }

        let current_index = self.current_index;
        let image_count = self.images.len() as isize;

        // 1. 現在の画像と前後CACHE_SIZE枚をキャッシュ対象とする。
        //    現在位置に近い順（0, +1, -1, +2, -2, ...）に並べることで、
        //    デコードの優先度が表示中の画像から順になるようにする。
        let mut retain_keys = std::collections::HashSet::new();
        let mut prefetch_order: Vec<PathBuf> = Vec::with_capacity(CACHE_SIZE * 2 + 1);
        let mut offsets: Vec<isize> = Vec::with_capacity(CACHE_SIZE * 2 + 1);
        offsets.push(0);
        for d in 1..=CACHE_SIZE as isize {
            offsets.push(d);
            offsets.push(-d);
        }
        for offset in offsets {
            let index = (current_index as isize + offset).rem_euclid(image_count) as usize;
            let path = self.images[index].clone();
            // 画像枚数がキャッシュ幅より少ない場合は同じパスが重複するため、初回のみ積む
            if retain_keys.insert(path.clone()) {
                prefetch_order.push(path);
            }
        }

        // 2. キャッシュ対象から外れたものをキャッシュから削除
        self.texture_cache.retain(|k, _| retain_keys.contains(k));

        // 3. 新しくキャッシュ対象になった画像を、同時実行数を制限しつつ読み込む。
        //    上限に達した分は、デコード完了時に update() から再度呼ばれて続きが走る。
        let mut budget = MAX_CONCURRENT_DECODES.saturating_sub(self.loading_images.len());
        for path in prefetch_order {
            if budget == 0 {
                break;
            }
            if self.texture_cache.contains_key(&path) || self.loading_images.contains(&path) {
                continue;
            }
            budget -= 1;
            self.loading_images.insert(path.clone());
            let tx = self.texture_sender.clone();
            let ctx_clone = ctx.clone();
            let generation = self.cache_generation;

            std::thread::spawn(move || {
                // 壊れた画像で image クレートが panic すると結果が送られず、
                // loading_images にパスが残り続けて永久に「読み込み中」になるため捕捉する
                let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    image::open(&path).map(|img| {
                        let (w, h) = (img.width(), img.height());
                        let rgba = img.to_rgba8();
                        let color_image = ColorImage::from_rgba_unmultiplied(
                            [w as usize, h as usize],
                            rgba.as_raw(),
                        );
                        (color_image, (w, h))
                    })
                }));
                let result = match decoded {
                    Ok(Ok(image)) => Ok(image),
                    Ok(Err(e)) => Err(format!("{e}")),
                    Err(_) => Err("デコードに失敗しました".to_string()),
                };
                let _ = tx.send(DecodedImage { path, generation, result });
                ctx_clone.request_repaint();
            });
        }
    }

    // スライドショーの開始/停止を設定する
    fn slide_show(&mut self, active: bool) {
        self.is_sliding = active;
        if active {
            self.slide_last = Instant::now(); // 開始時刻を記録
        }
    }

    // 画像の回転角度を絶対値で設定する
    fn rotate_image(&mut self, angle: f32) {
        self.rotation_angle = angle % 360.0;
    }

    // 画像の回転角度を相対的に変更する
    fn rotate_image_add(&mut self, delta: f32) {
        // 0-360の範囲に収まるように剰余を計算
        self.rotation_angle = (self.rotation_angle + delta).rem_euclid(360.0);
    }

    // 回転された画像をPNGとして上書き保存する
    fn save_png(&mut self, ctx: &egui::Context) {
        let Some(path) = self.current_file.clone() else {
            return;
        };
        // 拡張子をチェックし、PNGでなければ何もしない
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "png" {
            self.show_key("Save: PNG only");
            return;
        }
        // 回転していないなら書き込まない。
        // 無変換で save() すると再エンコードになり、元PNGのメタデータ
        // （テキストチャンク・gAMA など）が失われるため。
        let angle = (self.rotation_angle as i32).rem_euclid(360);
        if !matches!(angle, 90 | 180 | 270) {
            self.show_key("Save: 回転していません");
            return;
        }
        let was_sliding = self.is_sliding;
        if was_sliding {
            self.slide_show(false); // 保存中はスライドショーを一時停止
        }
        match image::open(&path) {
            Ok(img) => {
                // 角度に応じて回転処理
                let rotated = match angle {
                    90 => img.rotate90(),
                    180 => img.rotate180(),
                    _ => img.rotate270(),
                };
                match rotated.save(&path) {
                    Ok(_) => {
                        self.rotation_angle = 0.0; // 保存成功後、回転角度をリセット
                        // ファイルの中身が変わったので、キャッシュ済みテクスチャを捨てて
                        // 読み直す。これをしないと回転前の画像が表示されたままになる。
                        // 世代も進めて、保存前に走っていたデコード結果が後から
                        // 採用されて表示が巻き戻るのを防ぐ。
                        self.texture_cache.remove(&path);
                        self.cache_generation = self.cache_generation.wrapping_add(1);
                        self.current_texture = None;
                        self.current_image_size = None;
                        self.load_current_image(ctx);
                        self.show_key("Saved!");
                    }
                    Err(e) => self.show_key(&format!("Save error: {e}")),
                }
            }
            Err(e) => self.show_key(&format!("Load error: {e}")),
        }
        if was_sliding {
            self.slide_show(true); // スライドショーを再開
        }
    }

    // 短いメッセージを画面右下にオーバーレイ表示する
    /// 現在表示中のファイル本体をクリップボードにコピーする。
    /// ファイルマネージャ上で Ctrl+V / 貼り付けすると、ファイルとしてコピーされる。
    fn copy_file_to_clipboard(&mut self) {
        let Some(path) = self.current_file.clone() else { return; };
        if copy_file_to_clipboard_impl(&path, self.display_server) {
            self.show_key("ファイルをコピーしました");
        } else {
            self.show_key("ファイルのコピーに失敗しました");
        }
    }

    fn show_key(&mut self, text: &str) {
        self.key_overlay_text = text.to_string();
        self.key_overlay_hide_at =
            Some(Instant::now() + Duration::from_millis(KEY_OVERLAY_SHOW_MS));
    }

    // ウィンドウのタイトル文字列を生成する
    fn window_title(&self) -> String {
        let Some(ref path) = self.current_file else {
            return "rview".to_string();
        };
        // ファイル名
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // 親ディレクトリ名
        let dirname = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // 画像サイズ
        let size_str = if let Some((w, h)) = self.current_image_size {
            format!("({}x{})", w, h)
        } else {
            String::new()
        };
        // インデックスと総数
        let total = self.images.len();
        let index_str = if total > 0 {
            format!("{}/{}", self.current_index + 1, total)
        } else {
            String::new()
        };
        // スライドショーの状態
        let slide_str = if self.is_sliding { " [-Slide]" } else { "" };
        // 回転の状態
        let rot_str = if self.rotation_angle != 0.0 {
            format!(" [-Rotated: {}°]", self.rotation_angle)
        } else {
            String::new()
        };
        // これらを結合してタイトルを生成
        format!(
            "{}[{}]{} {}{}{}",
            filename, dirname, size_str, index_str, slide_str, rot_str
        )
    }

    // ファイルがドロップされたときの処理（egui 経由 — X11 で使用）
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // 右クリック（セカンダリボタン）中はドロップ処理をスキップ
        // コンテキストメニュー表示時にドロップイベントが誤発火するのを防ぐ
        let secondary_down = ctx.input(|i| i.pointer.secondary_down());
        if secondary_down {
            return;
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(first) = dropped.first() else {
            return; // ドロップされたファイルがなければ何もしない
        };
        let Some(ref path) = first.path else {
            return;
        };
        // 自分自身にドロップした場合（現在表示中のファイルと同じ）は無視
        if let Some(ref current) = self.current_file {
            if current == path {
                return;
            }
        }
        self.handle_dropped_path(&path.clone(), ctx);
    }

    // ドロップされたパスを処理する共通メソッド
    fn handle_dropped_path(&mut self, path: &Path, ctx: &egui::Context) {
        self.slide_show(false); // ドロップされたらスライドショーは停止

        if path.is_dir() {
            // ディレクトリがドロップされた場合
            self.init_images(path, None, ctx);
        } else if path.is_file() {
            // ファイルがドロップされた場合
            let dir = path.parent().unwrap_or(path).to_path_buf();
            self.init_images(&dir, Some(path), ctx);
            // ドロップ時はスライドショーを自動開始しない（表示するだけ）
        }
    }

    // メニューアクションを実行する（右クリックメニューとキーボードショートカットの共通処理）
    fn apply_menu_action(&mut self, action: MenuAction, ctx: &egui::Context) {
        match action {
            MenuAction::ToggleSlideShow => {
                let next = !self.is_sliding;
                self.slide_show(next);
                self.show_key(if next { "Slide ON" } else { "Slide OFF" });
            }
            MenuAction::ToggleFilename => {
                self.show_filename = !self.show_filename;
                self.show_key(if self.show_filename { "ファイル名を表示" } else { "ファイル名を非表示" });
            }
            MenuAction::ToggleSortOrder => {
                self.sort_order = if self.sort_order == SortOrder::Natural {
                    SortOrder::ModifiedDate
                } else {
                    SortOrder::Natural
                };
                let file = self.current_file.clone();
                self.apply_sort();
                if let Some(ref f) = file {
                    self.set_current_index_by_path(f);
                }
                let label = if self.sort_order == SortOrder::Natural { "自然順" } else { "更新日順" };
                self.show_key(label);
            }
            MenuAction::CopyFile => {
                self.copy_file_to_clipboard();
            }
            MenuAction::CopyFilePath => {
                if let Some(ref path) = self.current_file {
                    ctx.copy_text(path.to_string_lossy().into_owned());
                    self.show_key("ファイルパスをコピーしました");
                }
            }
            MenuAction::CopyFolderPath => {
                if let Some(ref path) = self.current_file {
                    if let Some(dir) = path.parent() {
                        ctx.copy_text(dir.to_string_lossy().into_owned());
                        self.show_key("フォルダパスをコピーしました");
                    }
                }
            }
            MenuAction::OpenFolder => {
                if let Some(ref path) = self.current_file {
                    if let Some(dir) = path.parent() {
                        open_folder(dir);
                        self.show_key("フォルダを開く");
                    }
                }
            }
            MenuAction::Rotate(angle) => {
                self.rotate_image(angle);
                self.show_key(&format!("Rotate {angle}°"));
            }
            MenuAction::Reload => {
                // 何も開いていない状態では current_dir が初期値 "./"（プロセスの
                // カレントディレクトリ）のままなので、F5 で意図しない場所を開かないよう無視する
                if self.current_file.is_none() {
                    return;
                }
                let dir = self.current_dir.clone();
                let file = self.current_file.clone();
                self.init_images(&dir, file.as_deref(), ctx);
                self.show_key("リロード");
            }
            MenuAction::ResizeWindow => {
                if let Some((w, h)) = self.current_image_size {
                    let angle_i = (self.rotation_angle as i32).rem_euclid(360);
                    let (eff_w, eff_h) = if angle_i == 90 || angle_i == 270 {
                        (h, w)
                    } else {
                        (w, h)
                    };
                    let mut size = egui::vec2(eff_w as f32, eff_h as f32);
                    // 画像がモニタより大きいとウィンドウが画面外へはみ出すのでアスペクト比を保って収める
                    if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                        let limit = monitor * 0.9;
                        if size.x > limit.x || size.y > limit.y {
                            let shrink = (limit.x / size.x).min(limit.y / size.y);
                            size *= shrink;
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                    self.show_key("ウインドウをリサイズ");
                }
            }
            MenuAction::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    // キーボード入力を処理する
    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        // 現在のキー入力状態をまとめて取得
        struct Keys {
            ctrl: bool,
            shift: bool,
            a: bool,
            d: bool,
            h: bool,
            j: bool,
            k: bool,
            l: bool,
            w: bool,
            s: bool,
            q: bool,
            e: bool,
            c: bool,
            f: bool,
            o: bool,
            r: bool,
            f5: bool,
            num1: bool,
            num2: bool,
            num3: bool,
            num4: bool,
            arrow_left: bool,
            arrow_right: bool,
            arrow_up: bool,
            arrow_down: bool,
            page_up: bool,
            page_down: bool,
            home: bool,
            end: bool,
            space: bool,
            escape: bool,
            copy_event: bool,
            scroll_y: f32,
        }

        let k = ctx.input(|i| Keys {
            ctrl: i.modifiers.ctrl,
            shift: i.modifiers.shift,
            a: i.key_pressed(egui::Key::A),
            d: i.key_pressed(egui::Key::D),
            h: i.key_pressed(egui::Key::H),
            j: i.key_pressed(egui::Key::J),
            k: i.key_pressed(egui::Key::K),
            l: i.key_pressed(egui::Key::L),
            w: i.key_pressed(egui::Key::W),
            s: i.key_pressed(egui::Key::S),
            q: i.key_pressed(egui::Key::Q),
            e: i.key_pressed(egui::Key::E),
            c: i.key_pressed(egui::Key::C),
            f: i.key_pressed(egui::Key::F),
            o: i.key_pressed(egui::Key::O),
            r: i.key_pressed(egui::Key::R),
            f5: i.key_pressed(egui::Key::F5),
            num1: i.key_pressed(egui::Key::Num1),
            num2: i.key_pressed(egui::Key::Num2),
            num3: i.key_pressed(egui::Key::Num3),
            num4: i.key_pressed(egui::Key::Num4),
            arrow_left: i.key_pressed(egui::Key::ArrowLeft),
            arrow_right: i.key_pressed(egui::Key::ArrowRight),
            arrow_up: i.key_pressed(egui::Key::ArrowUp),
            arrow_down: i.key_pressed(egui::Key::ArrowDown),
            page_up: i.key_pressed(egui::Key::PageUp),
            page_down: i.key_pressed(egui::Key::PageDown),
            home: i.key_pressed(egui::Key::Home),
            end: i.key_pressed(egui::Key::End),
            space: i.key_pressed(egui::Key::Space),
            escape: i.key_pressed(egui::Key::Escape),
            // egui-winit は Ctrl+C を Event::Copy に変換したうえで
            // Key::C のキーイベントを送出しないため、Copy イベント側で拾う
            copy_event: i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            scroll_y: i.raw_scroll_delta.y,
        });

        let has_images = !self.images.is_empty();

        // コピー系ショートカットは Copy イベントで判定する。
        // egui-winit は Ctrl+C / Ctrl+Shift+C を egui::Event::Copy に変換したうえで
        // early return するため、Key::C のキーイベントが一切届かない
        // （egui-winit の is_copy_command は shift を見ないので Ctrl+Shift+C も同様）。
        // そのため key_pressed(Key::C) では検出できない。
        if k.copy_event {
            if k.shift {
                self.apply_menu_action(MenuAction::CopyFilePath, ctx);
            } else {
                self.apply_menu_action(MenuAction::CopyFile, ctx);
            }
            return;
        }

        // Ctrl + Shift + キーのショートカット
        if k.ctrl && k.shift {
            // Ctrl+Shift+C は上の Copy イベント側で処理される。
            // ここは egui が Copy 変換をやめた場合のフォールバック
            if k.c { self.apply_menu_action(MenuAction::CopyFilePath, ctx); }
            if k.d { self.apply_menu_action(MenuAction::CopyFolderPath, ctx); }
            return; // Ctrl+Shiftショートカットが処理されたら他のキー処理はスキップ
        }

        // Ctrl + キーのショートカット (Shiftは押されていない)
        if k.ctrl && !k.shift {
            if k.arrow_up    { self.rotate_image(0.0);   self.show_key("Rotate 0°"); }
            if k.arrow_down  { self.rotate_image(180.0); self.show_key("Rotate 180°"); }
            if k.arrow_left  { self.rotate_image(270.0); self.show_key("Rotate 270°"); }
            if k.arrow_right { self.rotate_image(90.0);  self.show_key("Rotate 90°"); }
            if k.s           { self.save_png(ctx); }
            // Ctrl+C も上の Copy イベント側で処理される（ここはフォールバック）
            if k.c           { self.apply_menu_action(MenuAction::CopyFile, ctx); }
            if k.o           { self.apply_menu_action(MenuAction::OpenFolder, ctx); }
            if k.f           { self.apply_menu_action(MenuAction::ResizeWindow, ctx); }
            if k.r           { self.apply_menu_action(MenuAction::Reload, ctx); }
            if k.q           { self.apply_menu_action(MenuAction::Quit, ctx); }
            return; // Ctrlショートカットが処理されたら他のキー処理はスキップ
        }

        // Shift + キーのショートカット (Ctrlは押されていない)
        if k.shift && !k.ctrl {
            if has_images {
                if k.a || k.home { self.load_first_image(ctx); self.show_key("First"); }
                if k.d || k.end  { self.load_last_image(ctx);  self.show_key("Last"); }
                if k.w           { self.load_pgup_image(ctx);  self.show_key("PgUp"); }
                if k.s           { self.load_pgdn_image(ctx);  self.show_key("PgDn"); }
            }
            return; // Shiftショートカットが処理されたら他のキー処理はスキップ
        }

        // 修飾キーなしのショートカット
        if k.escape {
            self.slide_show(false);
            self.show_key("Stop Slide");
        }
        if k.space {
            self.apply_menu_action(MenuAction::ToggleSlideShow, ctx);
        }
        if k.f   { self.apply_menu_action(MenuAction::ToggleFilename, ctx); }
        if k.o   { self.apply_menu_action(MenuAction::ToggleSortOrder, ctx); }
        if k.f5  { self.apply_menu_action(MenuAction::Reload, ctx); }

        if k.num1 { self.rotate_image(0.0);   self.show_key("Rotate 0°"); }
        if k.num2 { self.rotate_image(90.0);  self.show_key("Rotate 90°"); }
        if k.num3 { self.rotate_image(180.0); self.show_key("Rotate 180°"); }
        if k.num4 { self.rotate_image(270.0); self.show_key("Rotate 270°"); }
        if k.q    { self.rotate_image_add(-90.0); self.show_key("Rotate -90°"); }
        if k.e    { self.rotate_image_add(90.0);  self.show_key("Rotate +90°"); }

        if has_images {
            // Vimライクなキーバインド + 矢印キー
            if k.a || k.arrow_left  || k.h || k.arrow_up   || k.k {
                self.shift_image(false, ctx);
                self.show_key("Prev");
            }
            if k.d || k.arrow_right || k.l || k.arrow_down || k.j {
                self.shift_image(true, ctx);
                self.show_key("Next");
            }
            if k.w || k.page_up   { self.load_pgup_image(ctx); self.show_key("PgUp"); }
            if k.s || k.page_down { self.load_pgdn_image(ctx); self.show_key("PgDn"); }
            if k.home { self.load_first_image(ctx); self.show_key("First"); }
            if k.end  { self.load_last_image(ctx);  self.show_key("Last"); }

            // マウスホイールでの画像切り替え（デバウンスで連射防止）
            let can_scroll = self.last_scroll_shift
                .map(|t| t.elapsed() >= Duration::from_millis(SCROLL_DEBOUNCE_MS))
                .unwrap_or(true);

            if k.scroll_y != 0.0 {
                // 前回の入力から間が空いていれば、独立したスクロール操作とみなして蓄積を捨てる
                let stale = self
                    .last_scroll_input
                    .map(|t| t.elapsed() >= Duration::from_millis(SCROLL_RESET_MS))
                    .unwrap_or(false);
                if stale {
                    self.scroll_accumulator = 0.0;
                }
                self.last_scroll_input = Some(Instant::now());
                self.scroll_accumulator += k.scroll_y;
            }

            if can_scroll {
                if self.scroll_accumulator > SCROLL_THRESHOLD {
                    self.shift_image(false, ctx); // 上スクロールで前の画像
                    self.last_scroll_shift = Some(Instant::now());
                    self.scroll_accumulator = 0.0;
                } else if self.scroll_accumulator < -SCROLL_THRESHOLD {
                    self.shift_image(true, ctx); // 下スクロールで次の画像
                    self.last_scroll_shift = Some(Instant::now());
                    self.scroll_accumulator = 0.0;
                }
            }
        }
    }

    // デスクトップ環境を検出してキャッシュする（Linux のみ）
    #[cfg(target_os = "linux")]
    fn detect_display_server(&mut self, frame: &eframe::Frame) {
        if self.display_server.is_some() {
            return; // 既に検出済み
        }
        use raw_window_handle::HasWindowHandle;
        let Some(handle) = frame.window_handle().ok() else { return };
        match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Xcb(_)
            | raw_window_handle::RawWindowHandle::Xlib(_) => {
                self.display_server = Some(DisplayServer::X11);
            }
            raw_window_handle::RawWindowHandle::Wayland(_) => {
                self.display_server = Some(DisplayServer::Wayland);
            }
            _ => {}
        }
    }

    // X11 ウィンドウIDを取得してキャッシュする（XDND用、Linux のみ）
    #[cfg(target_os = "linux")]
    fn get_x11_window_id(&mut self, frame: &eframe::Frame) -> Option<u32> {
        if let Some(id) = self.x11_window_id {
            return Some(id);
        }
        use raw_window_handle::HasWindowHandle;
        let id = match frame.window_handle().ok()?.as_raw() {
            raw_window_handle::RawWindowHandle::Xcb(h)  => h.window.get(),
            raw_window_handle::RawWindowHandle::Xlib(h) => h.window as u32,
            _ => return None,
        };
        self.x11_window_id = Some(id);
        Some(id)
    }

    // Wayland DnD コンテキストを初期化する（Linux のみ）
    #[cfg(target_os = "linux")]
    fn init_wayland_dnd(&mut self, frame: &eframe::Frame) {
        if self.wayland_dnd.is_some() {
            return;
        }
        use raw_window_handle::{HasWindowHandle, HasDisplayHandle};
        let Some(wh) = frame.window_handle().ok() else { return };
        let Some(dh) = frame.display_handle().ok() else { return };
        match (wh.as_raw(), dh.as_raw()) {
            (
                raw_window_handle::RawWindowHandle::Wayland(w),
                raw_window_handle::RawDisplayHandle::Wayland(d),
            ) => {
                match WaylandDndContext::new(d.display, w.surface) {
                    Some(ctx) => { self.wayland_dnd = Some(ctx); }
                    None => {
                        eprintln!("Wayland DnD の初期化に失敗しました");
                    }
                }
            }
            _ => {}
        }
    }

    // 画像を描画する
    fn draw_image(&mut self, ui: &mut egui::Ui) {
        let Some(ref texture) = self.current_texture else {
            // テクスチャがない場合
            ui.centered_and_justified(|ui| {
                if self.current_file.is_some() {
                    // ファイルはあるが読み込み中の場合
                    ui.add(egui::Spinner::new());
                    ui.label("読み込み中...");
                } else {
                    // ファイルが全くない場合
                    ui.label(
                        egui::RichText::new("ここに画像ファイルまたはフォルダをドロップしてください").size(24.0),
                    );
                }
            });
            return;
        };

        let tex_size = texture.size_vec2();
        let available_rect = ui.max_rect();
        let available = available_rect.size();

        // 90/270度回転時は、フィット計算のために実質的な幅と高さを入れ替える
        let angle_i = (self.rotation_angle as i32).rem_euclid(360);
        let (eff_w, eff_h) = if angle_i == 90 || angle_i == 270 {
            (tex_size.y, tex_size.x)
        } else {
            (tex_size.x, tex_size.y)
        };
        // 幅・高さのいずれかが 0 だと除算で inf / NaN になるので描画しない
        // （ウィンドウ最小化時や、サイズ 0 の画像を掴んだ場合）
        if eff_w <= 0.0 || eff_h <= 0.0 || available.x <= 0.0 || available.y <= 0.0 {
            return;
        }
        // 利用可能な領域に収まるようにスケールを計算
        let scale = (available.x / eff_w).min(available.y / eff_h);
        // fit_to_exact_size には元の（回転前の）アスペクト比でサイズを渡す
        // 逆アスペクト比を渡すと egui 内部で二重縮小が起きて画像が小さくなる
        let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);

        // ラジアンに変換してImageウィジェットに設定
        let angle_rad = self.rotation_angle.to_radians();
        let img = egui::Image::new((texture.id(), tex_size))
            .fit_to_exact_size(display_size)
            .rotate(angle_rad, egui::Vec2::splat(0.5)) // 中央を軸に回転
            .sense(egui::Sense::drag()); // ドラッグ可能にする

        // 画面中央に配置（回転の中心が画面中央になるようにする）
        let centered_rect = egui::Rect::from_center_size(available_rect.center(), display_size);
        let _response = ui.put(centered_rect, img);

        // ドラッグ開始の検知：押下しただけでは起動せず、ポインタが閾値以上移動してから
        // プラットフォーム固有の DnD を起動する（GTK と同じ方式）。
        // Sense::drag のみのウィジェットは押下と同じフレームで drag_started が発火するため、
        // その時点で start_drag すると単純クリックでも DnD セッションが発生してしまう。
        // Wayland ではこのファントムドラッグの後始末と次のドラッグ開始が競合し、
        // コンポジターがシリアル検証に失敗した start_drag を黙って無視することがある。
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            if !ui.input(|i| i.pointer.any_down()) {
                self.file_drag_triggered = false; // ボタンが離されたらジェスチャをリセット
            }
            if _response.dragged() && !self.file_drag_triggered && self.current_file.is_some() {
                let moved_enough = ui.input(|i| {
                    i.pointer
                        .press_origin()
                        .zip(i.pointer.latest_pos())
                        .is_some_and(|(origin, pos)| (pos - origin).length() >= DRAG_START_THRESHOLD)
                });
                if moved_enough {
                    self.pending_file_drag = true;
                    self.file_drag_triggered = true; // 1ジェスチャにつき1回だけ起動
                }
            }
        }
    }
}

// eframe::Appトレイトの実装
impl eframe::App for App {
    // 毎フレーム呼ばれる更新処理
    #[allow(unused_variables)]
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // デスクトップ環境を検出し、Wayland DnD イベントをディスパッチ（Linux のみ）
        #[cfg(target_os = "linux")]
        {
            self.detect_display_server(frame);

            // Wayland DnD イベントをディスパッチ
            if self.display_server == Some(DisplayServer::Wayland) {
                self.init_wayland_dnd(frame);
                if let Some(ref mut wl) = self.wayland_dnd {
                    wl.dispatch_pending();
                    wl.complete_pending_drops();
                    // 送信ドラッグ中、または受信ドラッグ（外部からのオファー受信／ドロップ待ち）中は
                    // 高頻度で再描画し、accept/set_actions/receive をコンポジターへ遅延なく届ける。
                    // KWin/KDE は応答が遅いとドロップをタイムアウトさせるため特に重要。
                    if wl.state.drag_active
                        || wl.state.current_offer.is_some()
                        || wl.state.pending_receive.is_some()
                    {
                        ctx.request_repaint_after(Duration::from_millis(16));
                    }
                }
                // Wayland DnD 受信で取得したファイルパスを処理
                if let Some(ref mut wl) = self.wayland_dnd {
                    let drops: Vec<PathBuf> = wl.state.received_drops.drain(..).collect();
                    for path in drops {
                        self.handle_dropped_path(&path, ctx);
                    }
                }
            }
        }

        self.handle_dropped_files(ctx); // ファイルドロップを処理（X11 / egui 経由）
        self.handle_keyboard(ctx);      // キーボード入力を処理

        // バックグラウンドスレッドからのデコード結果を処理
        // 1フレームで複数処理できるようにループさせる
        let mut decoded_any = false;
        while let Ok(DecodedImage { path, generation, result }) = self.texture_receiver.try_recv() {
            decoded_any = true;
            // 読み込み中セットからパスを削除（成否によらず必ず外す）
            self.loading_images.remove(&path);

            // 投入後に画像リストやファイル内容が変わっていれば、この結果はもう古い。
            // 採用すると表示とキャッシュが巻き戻るため捨てる（次の update_cache が読み直す）
            if generation != self.cache_generation {
                continue;
            }

            match result {
                Ok((color_image, (w, h))) => {
                    let name = path.to_string_lossy(); // テクスチャ名はユニークにする
                    let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);

                    // 現在表示すべきファイルなら、すぐに表示を更新
                    if self.current_file.as_ref() == Some(&path) {
                        self.current_texture = Some(texture.clone());
                        self.current_image_size = Some((w, h));
                    }
                    // キャッシュにテクスチャを保存
                    self.texture_cache.insert(path, texture);
                }
                Err(e) => {
                    // 現在のファイルでエラーが起きた場合のみメッセージを表示
                    if self.current_file.as_ref() == Some(&path) {
                        self.show_key(&format!("Error: {e}"));
                    }
                }
            }
        }
        // デコード枠が空いたので、残りのプリフェッチを進める。
        // 同時に、別ディレクトリを開いた後に到着した古い結果をキャッシュから追い出す。
        if decoded_any {
            self.update_cache(ctx);
        }

        // スライドショーの更新
        if self.is_sliding {
            if self.slide_last.elapsed() >= Duration::from_millis(SLIDE_SHOW_INTERVAL_MS) {
                self.shift_image(true, ctx); // 次の画像へ
                self.slide_last = Instant::now(); // 時刻を更新
            }
            // 次の更新を要求 (アニメーションのため)
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // キーオーバーレイの有効期限切れチェック。
        // egui は入力がないと再描画しないため、期限に合わせて再描画を予約しないと
        // オーバーレイが次の入力まで消えずに残ってしまう。
        if let Some(hide_at) = self.key_overlay_hide_at {
            let now = Instant::now();
            if now >= hide_at {
                self.key_overlay_text.clear();
                self.key_overlay_hide_at = None;
            } else {
                ctx.request_repaint_after(hide_at - now);
            }
        }

        // ウィンドウタイトルを更新（内容が変わったときだけ送る）
        let title = self.window_title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        // 中央パネルに画像とコンテキストメニューを描画
        let mut menu_action: Option<MenuAction> = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0))
            .show(ctx, |ui| {
            self.draw_image(ui); // 画像を描画

            // パネル全体で右クリックに反応するように設定
            let response = ui.interact(
                ui.max_rect(),
                egui::Id::new("context_menu_area"),
                egui::Sense::click(),
            );

            // クロージャ内で`self`を借用しないように、必要な状態をコピーしておく
            let is_sliding    = self.is_sliding;
            let show_filename = self.show_filename;
            let has_file      = self.current_file.is_some();
            let sort_order    = self.sort_order;

            response.context_menu(|ui| {
                // ラベルの右側にショートカットキーを添えたメニュー項目
                let item = |ui: &mut egui::Ui, label: &str, shortcut: &str| {
                    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
                };

                // スライドショー
                let slide_label = if is_sliding { "スライドショーを停止" } else { "スライドショーを開始" };
                if item(ui, slide_label, "Space") {
                    menu_action = Some(MenuAction::ToggleSlideShow);
                    ui.close_menu();
                }

                // ファイル名表示切替
                let fname_label = if show_filename { "ファイル名を非表示" } else { "ファイル名を表示" };
                if item(ui, fname_label, "F") {
                    menu_action = Some(MenuAction::ToggleFilename);
                    ui.close_menu();
                }

                // ソート順切替
                let sort_label = if sort_order == SortOrder::Natural { "更新日順にソート" } else { "自然順にソート" };
                if item(ui, sort_label, "O") {
                    menu_action = Some(MenuAction::ToggleSortOrder);
                    ui.close_menu();
                }

                ui.separator();

                // ファイル操作（画像読込済みのときのみ有効）
                ui.add_enabled_ui(has_file, |ui| {
                    if item(ui, "ファイルをコピー", "Ctrl+C") {
                        menu_action = Some(MenuAction::CopyFile);
                        ui.close_menu();
                    }
                    if item(ui, "ファイルパスをコピー", "Ctrl+Shift+C") {
                        menu_action = Some(MenuAction::CopyFilePath);
                        ui.close_menu();
                    }
                    if item(ui, "フォルダパスをコピー", "Ctrl+Shift+D") {
                        menu_action = Some(MenuAction::CopyFolderPath);
                        ui.close_menu();
                    }
                    if item(ui, "フォルダを開く", "Ctrl+O") {
                        menu_action = Some(MenuAction::OpenFolder);
                        ui.close_menu();
                    }
                });

                ui.separator();

                // 回転（画像読込済みのときのみ有効）
                ui.add_enabled_ui(has_file, |ui| {
                    if item(ui, "画像を0度回転",   "1 / Ctrl+↑") { menu_action = Some(MenuAction::Rotate(0.0));   ui.close_menu(); }
                    if item(ui, "画像を90度回転",  "2 / Ctrl+→") { menu_action = Some(MenuAction::Rotate(90.0));  ui.close_menu(); }
                    if item(ui, "画像を180度回転", "3 / Ctrl+↓") { menu_action = Some(MenuAction::Rotate(180.0)); ui.close_menu(); }
                    if item(ui, "画像を270度回転", "4 / Ctrl+←") { menu_action = Some(MenuAction::Rotate(270.0)); ui.close_menu(); }
                });

                ui.separator();

                // リロード
                ui.add_enabled_ui(has_file, |ui| {
                    if item(ui, "画像をリロード", "F5 / Ctrl+R") {
                        menu_action = Some(MenuAction::Reload);
                        ui.close_menu();
                    }
                    if item(ui, "画像のサイズに合わせてウインドウをリサイズ", "Ctrl+F") {
                        menu_action = Some(MenuAction::ResizeWindow);
                        ui.close_menu();
                    }
                });

                ui.separator();

                if item(ui, "アプリケーションを終了", "Ctrl+Q") {
                    menu_action = Some(MenuAction::Quit);
                    ui.close_menu();
                }
            });
        });

        // メニューアクションの処理
        if let Some(action) = menu_action {
            self.apply_menu_action(action, ctx);
        }

        // ファイルドラッグの開始（デスクトップ環境に応じた処理、Linux のみ）
        #[cfg(target_os = "linux")]
        if self.pending_file_drag {
            self.pending_file_drag = false;
            if let Some(path) = self.current_file.clone() {
                match self.display_server {
                    Some(DisplayServer::X11) => {
                        if let Some(win_id) = self.get_x11_window_id(frame) {
                            std::thread::spawn(move || xdnd_drag(win_id, path));
                        } else {
                            eprintln!("X11 Window ID not found.");
                        }
                    }
                    Some(DisplayServer::Wayland) => {
                        if let Some(ref mut wl) = self.wayland_dnd {
                            wl.start_drag(&path);
                        } else {
                            eprintln!("Wayland DnD context not initialized.");
                        }
                    }
                    None => {
                        eprintln!("Display server not detected. Drag and drop not available.");
                    }
                }
            }
        }

        // ファイルドラッグの開始（macOS のみ）
        #[cfg(target_os = "macos")]
        if self.pending_file_drag {
            self.pending_file_drag = false;
            if let Some(path) = self.current_file.clone() {
                macos_dnd::begin_file_drag(frame, &path);
            }
        }

        // ファイル名オーバーレイ（左下）
        if self.show_filename {
            if let Some(ref path) = self.current_file {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                egui::Area::new(egui::Id::new("filename_overlay"))
                    .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
                    .show(ctx, |ui| {
                        // ウィンドウ幅いっぱいまで使えるようにして折り返しを防ぐ
                        ui.set_max_width(ctx.screen_rect().width() - 20.0);
                        ui.label(
                            egui::RichText::new(name)
                                .size(14.0)
                                .color(egui::Color32::WHITE)
                                .background_color(egui::Color32::from_black_alpha(180)),
                        );
                    });
            }
        }

        // キー操作のオーバーレイ（右下）
        if !self.key_overlay_text.is_empty() {
            let text = self.key_overlay_text.clone();
            egui::Area::new(egui::Id::new("key_overlay"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-10.0, -10.0))
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(14.0)
                            .color(egui::Color32::WHITE)
                            .background_color(egui::Color32::from_black_alpha(180)),
                    );
                });
        }
    }
}

// リポジトリに同梱していないフォントの既定の配置場所
// （実行ファイルの隣、またはカレントディレクトリからの相対パス）
const BUNDLED_FONT_RELATIVE_PATH: &str =
    "assets/fonts/HackGen_NF_v2.10.0/HackGenConsoleNF-Regular.ttf";

// 日本語表示用フォントを探す。見つからなければ None を返す。
//
// フォントファイルはリポジトリに含めていないため、実行時に探索する。
// egui は内部で ab_glyph を使う関係で .ttc（フォントコレクション）を読めず、
// 渡すと set_fonts でパニックするため、候補は .ttf / .otf のみに絞る。
fn find_japanese_font() -> Option<Vec<u8>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. 環境変数 RVIEW_FONT による明示指定を最優先
    if let Some(path) = std::env::var_os("RVIEW_FONT") {
        candidates.push(PathBuf::from(path));
    }

    // 2. 実行ファイルと同じディレクトリ配下（配布時にフォントを同梱する場合）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(BUNDLED_FONT_RELATIVE_PATH));
        }
    }

    // 3. カレントディレクトリ配下（cargo run でリポジトリルートから起動した場合）
    candidates.push(PathBuf::from(BUNDLED_FONT_RELATIVE_PATH));

    // 4. システムにインストールされている日本語フォント
    candidates.extend(system_font_candidates().into_iter().map(PathBuf::from));

    for path in candidates {
        if !is_supported_font_file(&path) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes); // 存在しない候補は黙って次へ
        }
    }

    None
}

// egui が読める拡張子か（.ttc などのコレクション形式は読めない）
fn is_supported_font_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"),
        None => false,
    }
}

// プラットフォームごとのシステムフォント候補
// Windows/macOS の標準日本語フォントは .ttc のため候補にできず、
// ユーザーが RVIEW_FONT で指定するかフォントを同梱する運用になる。
fn system_font_candidates() -> Vec<&'static str> {
    if cfg!(target_os = "linux") {
        vec![
            "/usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf",
            "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
            "/usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf",
            "/usr/share/fonts/truetype/ipafont/ipagp.ttf",
            "/usr/share/fonts/truetype/ipafont-gothic/ipagp.ttf",
        ]
    } else {
        Vec::new()
    }
}

// カスタムフォントをセットアップする
//
// フォントが見つからない場合は egui の既定フォントのまま続行する。
// その場合、日本語は表示できない（豆腐になる）が、起動自体は妨げない。
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let Some(bytes) = find_japanese_font() else {
        eprintln!(
            "日本語フォントが見つかりませんでした。日本語を表示するには、\n\
             .ttf/.otf フォントを {BUNDLED_FONT_RELATIVE_PATH} に配置するか、\n\
             環境変数 RVIEW_FONT にフォントファイルのパスを指定してください。"
        );
        ctx.set_fonts(fonts);
        return;
    };

    fonts.font_data.insert(
        "my_font".to_owned(),
        egui::FontData::from_owned(bytes).into(),
    );

    // Proportional (プロポーショナル) フォントファミリーの先頭にカスタムフォントを追加
    // これにより、UIのデフォルトフォントとして優先的に使用されます。
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "my_font".to_owned());

    // Monospace (等幅) フォントファミリーにもカスタムフォントを追加
    // `TextEdit`などで等幅フォントが指定された場合に使用されます。
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("my_font".to_owned());

    ctx.set_fonts(fonts);
}

// 自然順ソート: 文字列をアルファベット部分と数字部分に分けて比較する
// 例: "file2.txt" は "file10.txt" より前に来る
//
// 大文字小文字とゼロ埋めの違いは無視して比較するが、それ以外が完全に同じ場合は
// Equal を返さず元の文字で決着させる。Equal を返すと "A.png" と "a.png" の順序が
// ディレクトリの読み取り順まかせになり、表示順が実行ごとに変わってしまうため。
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    // 大文字小文字 / ゼロ埋めの差。他が全て同じだったときだけ使う
    let mut tiebreak = Ordering::Equal;
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return tiebreak,     // 両方末尾。差はタイブレークのみ
            (None, _) => return Ordering::Less,  // aが先に尽きたらaが小さい
            (_, None) => return Ordering::Greater, // bが先に尽きたらaが大きい
            (Some(ac), Some(bc)) => {
                // 両方の文字が数字の場合
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    let an = collect_digits(&mut ai); // aから連続する数字を収集
                    let bn = collect_digits(&mut bi); // bから連続する数字を収集
                    // 桁数 → 辞書順の順で比べる。先頭の 0 を落としてあるので
                    // これは桁あふれなしの正確な数値比較になる
                    let asig = an.trim_start_matches('0');
                    let bsig = bn.trim_start_matches('0');
                    match asig.len().cmp(&bsig.len()).then_with(|| asig.cmp(bsig)) {
                        Ordering::Equal => {
                            // 数値としては同じ。"01" と "1" のようなゼロ埋め差は
                            // ゼロ埋めの多い方を先にする（GNU sort -V と同じ順序）。
                            // タイブレークなので、他に差があればそちらが優先される
                            if tiebreak == Ordering::Equal {
                                tiebreak = bn.len().cmp(&an.len());
                            }
                        }
                        other => return other, // 数字が違えばその結果を返す
                    }
                } else {
                    // 数字でない場合は文字として比較 (大文字小文字を区別しない)
                    let al = ac.to_ascii_lowercase();
                    let bl = bc.to_ascii_lowercase();
                    match al.cmp(&bl) {
                        Ordering::Equal => {
                            // 大文字小文字だけの違いはタイブレークとして保留
                            if tiebreak == Ordering::Equal && ac != bc {
                                tiebreak = ac.cmp(&bc);
                            }
                            ai.next();
                            bi.next();
                        }
                        other => return other, // 文字が違えばその結果を返す
                    }
                }
            }
        }
    }
}

// 指定されたパスをOSのデフォルトのファイルエクスプローラーで開く
fn open_folder(path: &Path) {
    #[cfg(target_os = "linux")]
    { std::process::Command::new("xdg-open").arg(path).spawn().ok(); }
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(path).spawn().ok(); }
    #[cfg(target_os = "macos")]
    { std::process::Command::new("open").arg(path).spawn().ok(); }
}

/// ファイル本体をクリップボードに載せる（ファイルマネージャに貼り付け可能な形式）。
/// 成功したら true を返す。プラットフォームごとに方式が異なる。
fn copy_file_to_clipboard_impl(path: &Path, display_server: Option<DisplayServer>) -> bool {
    #[cfg(target_os = "linux")]
    {
        // file URI を生成（DnD と同じく url クレートで正しくエンコード）
        let uri = url::Url::from_file_path(path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", path.display()));
        // GNOME 系ファイラー（Nautilus / Nemo / Caja / Thunar / Dolphin 互換）は
        // x-special/gnome-copied-files を見る。形式は "copy\n<uri>"。
        let gnome_data = format!("copy\n{uri}");
        match display_server {
            Some(DisplayServer::Wayland) => copy_via_stdin(
                "wl-copy",
                &["--type", "x-special/gnome-copied-files"],
                gnome_data.as_bytes(),
            ),
            // X11 もしくは未検出時は xclip を使用
            _ => copy_via_stdin(
                "xclip",
                &["-selection", "clipboard", "-t", "x-special/gnome-copied-files"],
                gnome_data.as_bytes(),
            ),
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = display_server;
        // PowerShell の Set-Clipboard -LiteralPath で CF_HDROP をセットする
        let p = path.display().to_string().replace('\'', "''");
        let script = format!("Set-Clipboard -LiteralPath '{p}'");
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = display_server;
        // AppleScript でファイル参照（POSIX file）をクリップボードに設定する
        let p = path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("set the clipboard to (POSIX file \"{p}\")");
        std::process::Command::new("osascript")
            .args(["-e", &script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    // 上記以外のプラットフォームでは未対応（戻り値の型が消えてビルドが壊れるのを防ぐ）
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = (path, display_server);
        false
    }
}

/// 外部コマンドの stdin にデータを書き込んでクリップボードにコピーする（Linux 用）。
/// wl-copy / xclip はクリップボード内容を保持するため自身でデーモン化するので、
/// `wait` はすぐに返る。
#[cfg(target_os = "linux")]
fn copy_via_stdin(cmd: &str, args: &[&str], data: &[u8]) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new(cmd).args(args).stdin(Stdio::piped()).spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(data).is_err() {
            return false;
        }
        // ここで stdin が drop され、書き込み端が閉じる
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

// ============================================================
// macOS Drag and Drop（ファイルドラッグ送出）実装
// 表示中の画像をFinder等へドラッグ＆ドロップでコピーする
// ============================================================

#[cfg(target_os = "macos")]
mod macos_dnd {
    use std::ffi::{c_char, c_void, CString};
    use std::path::Path;
    use std::sync::Once;

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize { width: f64, height: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect { origin: CGPoint, size: CGSize }

    // Objective-C ランタイム FFI
    // macOS では libobjc は常にリンクされている（eframe/winit 経由）
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *const c_void;
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
        fn objc_allocateClassPair(
            superclass: *const c_void, name: *const c_char, extra: usize,
        ) -> *mut c_void;
        fn objc_registerClassPair(cls: *mut c_void);
        fn class_addMethod(
            cls: *mut c_void, sel: *const c_void, imp: *const c_void, types: *const c_char,
        ) -> bool;
        fn class_addProtocol(cls: *mut c_void, protocol: *const c_void) -> bool;
        fn objc_getProtocol(name: *const c_char) -> *const c_void;
        fn objc_release(obj: *const c_void);
    }

    // objc_msgSend を適切な関数シグネチャにキャストして呼び出すヘルパー

    /// [obj selector] → id
    unsafe fn msg0(obj: *const c_void, sel: *const c_void) -> *const c_void {
        let f: unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void
            = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel)
    }

    /// [obj selector: a1] → id
    unsafe fn msg1(obj: *const c_void, sel: *const c_void, a1: *const c_void) -> *const c_void {
        let f: unsafe extern "C" fn(*const c_void, *const c_void, *const c_void) -> *const c_void
            = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel, a1)
    }

    /// [obj selector: a1 key2: a2 key3: a3] → id
    unsafe fn msg3(
        obj: *const c_void, sel: *const c_void,
        a1: *const c_void, a2: *const c_void, a3: *const c_void,
    ) -> *const c_void {
        let f: unsafe extern "C" fn(
            *const c_void, *const c_void, *const c_void, *const c_void, *const c_void,
        ) -> *const c_void
            = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel, a1, a2, a3)
    }

    /// [obj setSize: size] (CGSize 引数、void 返却)
    unsafe fn msg_set_size(obj: *const c_void, sel: *const c_void, size: CGSize) {
        let f: unsafe extern "C" fn(*const c_void, *const c_void, CGSize)
            = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel, size)
    }

    /// [obj setDraggingFrame: rect contents: contents] (CGRect + id 引数、void 返却)
    unsafe fn msg_set_frame(
        obj: *const c_void, sel: *const c_void, rect: CGRect, contents: *const c_void,
    ) {
        let f: unsafe extern "C" fn(*const c_void, *const c_void, CGRect, *const c_void)
            = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f(obj, sel, rect, contents)
    }

    /// よく使うセレクタを取得
    unsafe fn sel(name: &CString) -> *const c_void {
        sel_registerName(name.as_ptr())
    }

    /// NSDragOperationCopy = 1
    const NS_DRAG_OPERATION_COPY: usize = 1;

    /// NSDraggingSource プロトコルの必須メソッド実装
    /// draggingSession:sourceOperationMaskForDraggingContext: → NSDragOperationCopy を返す
    unsafe extern "C" fn drag_source_operation_mask(
        _self: *const c_void, _sel: *const c_void,
        _session: *const c_void, _context: isize,
    ) -> usize {
        NS_DRAG_OPERATION_COPY
    }

    static INIT: Once = Once::new();
    static mut DRAG_SOURCE_CLASS: *const c_void = std::ptr::null();

    /// NSDraggingSource プロトコルに準拠するミニマルな Objective-C クラスを動的に登録する
    unsafe fn ensure_drag_source_class() {
        INIT.call_once(|| {
            let superclass = objc_getClass(c"NSObject".as_ptr());
            let name = c"RviewDragSource";
            let cls = objc_allocateClassPair(superclass, name.as_ptr(), 0);
            if cls.is_null() {
                // 既に登録済み（ホットリロード等）の場合は既存クラスを取得
                DRAG_SOURCE_CLASS = objc_getClass(name.as_ptr());
                return;
            }

            // NSDraggingSource プロトコルを追加
            let protocol = objc_getProtocol(c"NSDraggingSource".as_ptr());
            if !protocol.is_null() {
                class_addProtocol(cls, protocol);
            }

            // 必須メソッドを追加:
            // - (NSDragOperation)draggingSession:(NSDraggingSession *)session
            //         sourceOperationMaskForDraggingContext:(NSDraggingContext)context;
            // 型エンコーディング: Q@:@q （返値:NSUInteger, self, _cmd, session, context:NSInteger）
            let method_sel = sel_registerName(
                c"draggingSession:sourceOperationMaskForDraggingContext:".as_ptr(),
            );
            class_addMethod(
                cls,
                method_sel,
                drag_source_operation_mask as *const c_void,
                c"Q@:@q".as_ptr(),
            );

            objc_registerClassPair(cls);
            DRAG_SOURCE_CLASS = cls as *const c_void;
        });
    }

    /// macOS ネイティブのドラッグセッションを開始する
    ///
    /// NSView.beginDraggingSessionWithItems:event:source: を呼び出し、
    /// 表示中のファイルを他のアプリケーションへドラッグ＆ドロップできるようにする。
    pub fn begin_file_drag(frame: &eframe::Frame, file_path: &Path) {
        use raw_window_handle::HasWindowHandle;

        let Some(path_str) = file_path.to_str() else { return };
        let Ok(handle) = frame.window_handle() else { return };
        let ns_view = match handle.as_raw() {
            raw_window_handle::RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as *const c_void,
            _ => return,
        };

        // セレクタ文字列を事前に準備
        let sel_new = CString::new("new").unwrap();
        let sel_string_utf8 = CString::new("stringWithUTF8String:").unwrap();
        let sel_file_url = CString::new("fileURLWithPath:").unwrap();
        let sel_alloc = CString::new("alloc").unwrap();
        let sel_init_pw = CString::new("initWithPasteboardWriter:").unwrap();
        let sel_shared_ws = CString::new("sharedWorkspace").unwrap();
        let sel_icon = CString::new("iconForFile:").unwrap();
        let sel_set_size = CString::new("setSize:").unwrap();
        let sel_set_frame = CString::new("setDraggingFrame:contents:").unwrap();
        let sel_array_with = CString::new("arrayWithObject:").unwrap();
        let sel_shared_app = CString::new("sharedApplication").unwrap();
        let sel_cur_event = CString::new("currentEvent").unwrap();
        let sel_begin_drag = CString::new("beginDraggingSessionWithItems:event:source:").unwrap();

        unsafe {
            ensure_drag_source_class();
            if DRAG_SOURCE_CLASS.is_null() { return; }

            // ドラッグソースインスタンスを生成 (retained: +1)
            let source = msg0(DRAG_SOURCE_CLASS, sel(&sel_new));

            // ファイルパスから NSString を生成 (autoreleased)
            let path_cstr = CString::new(path_str).unwrap();
            let ns_string = msg1(
                objc_getClass(c"NSString".as_ptr()),
                sel(&sel_string_utf8),
                path_cstr.as_ptr() as *const c_void,
            );

            // NSURL fileURLWithPath: (autoreleased)
            let file_url = msg1(
                objc_getClass(c"NSURL".as_ptr()),
                sel(&sel_file_url),
                ns_string,
            );

            // NSDraggingItem alloc + initWithPasteboardWriter: (retained: +1)
            let dragging_item = msg1(
                msg0(
                    objc_getClass(c"NSDraggingItem".as_ptr()),
                    sel(&sel_alloc),
                ),
                sel(&sel_init_pw),
                file_url,
            );

            // ファイルアイコンを取得して 32x32 に設定
            let workspace = msg0(
                objc_getClass(c"NSWorkspace".as_ptr()),
                sel(&sel_shared_ws),
            );
            let icon = msg1(workspace, sel(&sel_icon), ns_string);
            let icon_size = CGSize { width: 32.0, height: 32.0 };
            msg_set_size(icon, sel(&sel_set_size), icon_size);

            // NSDraggingItem にフレームとアイコンを設定
            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: icon_size,
            };
            msg_set_frame(dragging_item, sel(&sel_set_frame), rect, icon);

            // NSArray に格納 (autoreleased)
            let items = msg1(
                objc_getClass(c"NSArray".as_ptr()),
                sel(&sel_array_with),
                dragging_item,
            );

            // NSApp から現在のイベントを取得
            let app = msg0(
                objc_getClass(c"NSApplication".as_ptr()),
                sel(&sel_shared_app),
            );
            let event = msg0(app, sel(&sel_cur_event));
            if event.is_null() {
                objc_release(source);
                objc_release(dragging_item);
                return;
            }

            // ドラッグセッションを開始
            msg3(ns_view, sel(&sel_begin_drag), items, event, source);

            // retained オブジェクトを解放
            objc_release(source);
            objc_release(dragging_item);
        }
    }
}

// ============================================================
// X11 XDND（X Drag and Drop）プロトコル実装
// 表示中の画像をファイルマネージャー等へドラッグ＆ドロップでコピーする
// ============================================================

/// XDND で使用するアトムをまとめた構造体
#[cfg(target_os = "linux")]
struct XdndAtoms {
    xdnd_aware:       u32,
    xdnd_enter:       u32,
    xdnd_position:    u32,
    xdnd_status:      u32,
    xdnd_leave:       u32,
    xdnd_drop:        u32,
    xdnd_finished:    u32,
    xdnd_selection:   u32,
    xdnd_action_copy: u32,
    uri_list:         u32,  // "text/uri-list"
}

/// 必要なアトムを一括インターン
#[cfg(target_os = "linux")]
fn intern_xdnd_atoms(conn: &x11rb::rust_connection::RustConnection)
    -> Result<XdndAtoms, Box<dyn std::error::Error>>
{
    use x11rb::protocol::xproto::ConnectionExt as _;
    macro_rules! atom {
        ($name:expr) => { conn.intern_atom(false, $name.as_bytes())?.reply()?.atom }
    }
    Ok(XdndAtoms {
        xdnd_aware:       atom!("XdndAware"),
        xdnd_enter:       atom!("XdndEnter"),
        xdnd_position:    atom!("XdndPosition"),
        xdnd_status:      atom!("XdndStatus"),
        xdnd_leave:       atom!("XdndLeave"),
        xdnd_drop:        atom!("XdndDrop"),
        xdnd_finished:    atom!("XdndFinished"),
        xdnd_selection:   atom!("XdndSelection"),
        xdnd_action_copy: atom!("XdndActionCopy"),
        uri_list:         atom!("text/uri-list"),
    })
}

/// カーソル下で XdndAware なウィンドウを探す。
/// `exclude` には自分のウィンドウ（ドラッグ元のダミーウィンドウと、
/// 実際に表示しているメインウィンドウの両方）を渡してドロップ先候補から外す。
#[cfg(target_os = "linux")]
fn find_xdnd_target(conn: &x11rb::rust_connection::RustConnection, root: u32, exclude: &[u32], atoms: &XdndAtoms) -> u32 {
    use x11rb::protocol::xproto::{ConnectionExt as _, AtomEnum};
    let mut window = root;
    let mut last_aware = 0u32;
    loop {
        if !exclude.contains(&window) {
            if let Some(prop) = conn.get_property(false, window, atoms.xdnd_aware,
                    AtomEnum::ANY, 0, 1).ok().and_then(|c| c.reply().ok()) {
                if prop.value_len > 0 { last_aware = window; }
            }
        }
        match conn.query_pointer(window).ok().and_then(|c| c.reply().ok()) {
            Some(p) if p.child != 0 && p.child != window => window = p.child,
            _ => break,
        }
    }
    last_aware
}

/// ClientMessage を target ウィンドウへ送信
#[cfg(target_os = "linux")]
fn xdnd_send(conn: &x11rb::rust_connection::RustConnection, target: u32, type_: u32, d: [u32; 5]) {
    use x11rb::protocol::xproto::{ConnectionExt as _, EventMask, ClientMessageEvent, ClientMessageData};
    let _ = conn.send_event(false, target, EventMask::NO_EVENT, ClientMessageEvent {
        response_type: 33, // CLIENT_MESSAGE
        format: 32,
        sequence: 0,
        window: target,
        type_,
        data: ClientMessageData::from(d),
    });
}

/// SelectionRequest に対して text/uri-list データで応答
#[cfg(target_os = "linux")]
fn xdnd_reply_selection(
    conn: &x11rb::rust_connection::RustConnection,
    req:  &x11rb::protocol::xproto::SelectionRequestEvent,
    data: &[u8],
    atoms: &XdndAtoms,
) {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, EventMask, SelectionNotifyEvent, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    // リクエストされたフォーマットが text/uri-list のときのみデータを提供
    let property = if req.target == atoms.uri_list && req.property != 0 {
        let _ = conn.change_property8(PropMode::REPLACE, req.requestor,
                                      req.property, atoms.uri_list, data);
        req.property
    } else {
        0 // ATOM_NONE = 提供できないフォーマット
    };
    let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, SelectionNotifyEvent {
        response_type: 31, // SELECTION_NOTIFY
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property,
    });
    let _ = conn.flush();
}

#[cfg(target_os = "linux")]
/// X11 XDND ドラッグ操作（バックグラウンドスレッドから呼ぶ）
///
/// 流れ：
///   1. 別途 X11 接続を作成
///   2. 自ウィンドウに XdndAware v5 をセット・XdndSelection 所有権取得
///   3. ポーリングループ：カーソル追跡 → XdndEnter/Position/Leave 送信
///   4. マウスボタン離した瞬間に XdndDrop または XdndLeave 送信
///   5. XdndFinished 受信で終了
fn xdnd_drag(src_window: u32, file_path: PathBuf) {
    use x11rb::protocol::{xproto::{ConnectionExt as _, PropMode, AtomEnum}, Event as XEvent};
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;
    use x11rb::protocol::xproto::CreateWindowAux;
    use x11rb::connection::Connection;

    // RAII ガードで、ドラッグ終了時にカーソルを元に戻す
    struct CursorGuard<'a, C: Connection> {
        conn: &'a C,
        window: u32,
        cursor: u32,
    }

    impl<'a, C: Connection> Drop for CursorGuard<'a, C> {
        fn drop(&mut self) {
            // カーソルを元に戻す (None/0 を設定すると親ウィンドウのカーソルを継承)
            let _ = self.conn.change_window_attributes(
                self.window,
                &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().cursor(x11rb::NONE),
            );
            // 作成したカーソルを解放
            let _ = self.conn.free_cursor(self.cursor);
            let _ = self.conn.flush();
        }
    }

    // DISPLAY 環境変数から X11 接続を確立（eframe とは別コネクション）
    let Ok((conn, _)) = RustConnection::connect(None) else { return; };

    // "grabbing" カーソル (XC_grabbing = 60) を作成
    let grabbing_cursor = conn.generate_id().unwrap();
    let cursor_font = conn.generate_id().unwrap();
    let _ = conn.open_font(cursor_font, b"cursor");
    let _ = conn.create_glyph_cursor(
        grabbing_cursor, cursor_font, cursor_font,
        60, 61, // XC_grabbing, XC_grabbing+1
        0, 0, 0, 65535, 65535, 65535, // black and white
    );
    let _ = conn.close_font(cursor_font);

    // ガードオブジェクトを作成。この関数のスコープを抜けるときに drop が呼ばれる
    let _cursor_guard = CursorGuard { conn: &conn, window: src_window, cursor: grabbing_cursor };

    let Ok(atoms) = intern_xdnd_atoms(&conn) else { return; };

    // ダミーウィンドウを作成してドラッグ元とする
    // (src_windowの所有権は別スレッド(winit)にあるため、SelectionRequestを受け取れない)
    let root = conn.setup().roots[0].root;
    let dummy_window = conn.generate_id().unwrap();
    let _ = conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        dummy_window,
        root,
        0, 0, 1, 1, 0,
        x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    );

    // XdndAware プロパティをセット（バージョン 5）(ダミーウィンドウに対して)
    let _ = conn.change_property32(PropMode::REPLACE, dummy_window,
                                   atoms.xdnd_aware, AtomEnum::ATOM, &[5]);
    // XdndSelection の所有権を取得（この接続で SelectionRequest を受け取れるようになる）
    let _ = conn.set_selection_owner(dummy_window, atoms.xdnd_selection, 0u32);
    let _ = conn.flush();

    // メインウィンドウのカーソルを「掴む」アイコンに変更
    let _ = conn.change_window_attributes(
        src_window,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().cursor(grabbing_cursor),
    );
    let _ = conn.flush();

    // ファイル URI（text/uri-list 形式）
    // urlクレートを使って正しくエンコードする
    let uri_str = url::Url::from_file_path(&file_path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{}", file_path.display()));
    let uri_list = format!("{}\r\n", uri_str);

    let mut cur_target = 0u32;
    let mut can_drop   = false;

    loop {
        // カーソル位置とマウスボタン状態を取得
        let Some(ptr) = conn.query_pointer(src_window).ok().and_then(|c| c.reply().ok()) else { break; };
        let btn_down = u16::from(ptr.mask) & 0x100 != 0; // Button1Mask

        // カーソル下の XdndAware ウィンドウを探す
        let new_target = find_xdnd_target(&conn, ptr.root, &[dummy_window, src_window], &atoms);

        if new_target != cur_target {
            if cur_target != 0 {
                // 旧ターゲットを離れる
                xdnd_send(&conn, cur_target, atoms.xdnd_leave, [dummy_window, 0, 0, 0, 0]);
            }
            cur_target = new_target;
            can_drop   = false;
            if cur_target != 0 {
                // 新ターゲットへ入る（サポートするMIMEタイプを通知）
                xdnd_send(&conn, cur_target, atoms.xdnd_enter,
                          [dummy_window, 5 << 24, atoms.uri_list, 0, 0]);
            }
        }

        if cur_target != 0 {
            // 現在位置を通知
            let pos = ((ptr.root_x as u16 as u32) << 16) | (ptr.root_y as u16 as u32);
            xdnd_send(&conn, cur_target, atoms.xdnd_position,
                      [dummy_window, 0, pos, 0, atoms.xdnd_action_copy]);
        }

        // 受信イベントを処理（ノンブロッキング）
        let _ = conn.flush();
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                XEvent::SelectionRequest(req) => {
                    xdnd_reply_selection(&conn, &req, uri_list.as_bytes(), &atoms);
                }
                XEvent::ClientMessage(msg) => {
                    let d = msg.data.as_data32();
                    if msg.type_ == atoms.xdnd_status {
                        can_drop = d[1] & 1 != 0; // ターゲットがドロップを受け入れるか
                    } else if msg.type_ == atoms.xdnd_finished {
                        return; // ドロップ完了
                    }
                }
                _ => {}
            }
        }

        if !btn_down {
            // マウスボタンが離された
            // 直前に送った XdndPosition への XdndStatus がまだ届いていない可能性がある。
            // 素早いドラッグではターゲットの応答前にボタンが離されるため、少し待って判定する
            if cur_target != 0 && !can_drop {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
                while std::time::Instant::now() < deadline && !can_drop {
                    match conn.poll_for_event() {
                        Ok(Some(XEvent::SelectionRequest(req))) => {
                            xdnd_reply_selection(&conn, &req, uri_list.as_bytes(), &atoms);
                        }
                        Ok(Some(XEvent::ClientMessage(msg))) => {
                            if msg.type_ == atoms.xdnd_status {
                                can_drop = msg.data.as_data32()[1] & 1 != 0;
                            }
                        }
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(8)),
                        Err(_) => break,
                        _ => {}
                    }
                }
            }
            if cur_target != 0 && can_drop {
                // 自分自身へのドロップは無視する
                if cur_target == src_window {
                    xdnd_send(&conn, cur_target, atoms.xdnd_leave, [dummy_window, 0, 0, 0, 0]);
                    let _ = conn.flush();
                    return;
                }

                xdnd_send(&conn, cur_target, atoms.xdnd_drop, [dummy_window, 0, 0, 0, 0]);
                let _ = conn.flush();
                // XdndFinished を最大 3 秒待つ（その間 SelectionRequest にも応答する）
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while std::time::Instant::now() < deadline {
                    match conn.poll_for_event() {
                        Ok(Some(XEvent::SelectionRequest(req))) => {
                            xdnd_reply_selection(&conn, &req, uri_list.as_bytes(), &atoms);
                        }
                        Ok(Some(XEvent::ClientMessage(msg)))
                            if msg.type_ == atoms.xdnd_finished => { return; }
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(16)),
                        Err(_) => break,
                        _ => {}
                    }
                }
            } else if cur_target != 0 {
                xdnd_send(&conn, cur_target, atoms.xdnd_leave, [dummy_window, 0, 0, 0, 0]);
                let _ = conn.flush();
            }
            return;
        }

        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

// ============================================================
// Wayland DnD（Drag and Drop）実装
// 表示中の画像をファイルマネージャー等へドラッグ＆ドロップでコピーする
// ============================================================

#[cfg(target_os = "linux")]
use wayland_client::protocol::{
    wl_registry, wl_seat, wl_pointer,
    wl_data_device, wl_data_device_manager, wl_data_source,
    wl_data_offer, wl_surface, wl_compositor, wl_shm, wl_buffer,
};
#[cfg(target_os = "linux")]
use wayland_client::{Connection as WlConnection, Dispatch as WlDispatch, QueueHandle as WlQueueHandle, Proxy};

/// Wayland DnD の状態を保持する構造体
#[cfg(target_os = "linux")]
struct WaylandDndState {
    last_button_serial: u32,    // 最後のポインタボタンイベントのシリアル
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    compositor: Option<wl_compositor::WlCompositor>, // ドラッグアイコン用サーフェス作成
    shm: Option<wl_shm::WlShm>,                     // カーソルテーマ読み込み用
    drag_uri: Option<String>,   // ドラッグ中のファイルURI
    drag_active: bool,          // ドラッグ操作中かどうか
    drag_icon_surface: Option<wl_surface::WlSurface>, // ドラッグ中のアイコンサーフェス
    // DnD 受信用の状態（winit が Wayland DnD 受信を未実装のため自前で処理）
    current_offer: Option<wl_data_offer::WlDataOffer>, // 現在のドラッグオファー
    offer_has_uri_list: bool,   // オファーが text/uri-list を含むか
    // ドロップデータの読み取り用パイプと、対応する data_offer。
    // finish()/destroy() はデータ読み取り完了後に呼ぶ必要があるため offer を持ち越す
    // （KWin/KDE は DnD v3 の手順を厳格に検証するため、転送前の finish/destroy でドロップが失敗する）
    pending_receive: Option<PendingReceive>,
    received_drops: Vec<PathBuf>, // 受信完了したドロップファイルパス
}

/// 進行中のドロップデータ受信。
/// パイプはノンブロッキングにしてフレームをまたいで少しずつ読む。
/// ブロッキング読み取りにすると、ドラッグ元が応答しない場合に UI スレッドが固まる。
#[cfg(target_os = "linux")]
struct PendingReceive {
    stream: std::os::unix::net::UnixStream,
    offer: wl_data_offer::WlDataOffer,
    buf: Vec<u8>,
    deadline: Instant, // これを過ぎたら諦めて offer を破棄する
}

/// ドロップデータ受信を諦めるまでの上限時間
#[cfg(target_os = "linux")]
const DROP_RECEIVE_TIMEOUT: Duration = Duration::from_secs(3);

/// Wayland DnD コンテキスト
#[cfg(target_os = "linux")]
struct WaylandDndContext {
    conn: WlConnection,
    event_queue: wayland_client::EventQueue<WaylandDndState>,
    state: WaylandDndState,
    origin_surface: wl_surface::WlSurface, // eframe のサーフェスをラップしたもの
    cursor_theme: Option<wayland_cursor::CursorTheme>, // ドラッグアイコン用（共有メモリを保持）
}

#[cfg(target_os = "linux")]
impl WaylandDndContext {
    /// eframe の display/surface ポインタから DnD コンテキストを初期化する
    fn new(
        display_ptr: std::ptr::NonNull<std::ffi::c_void>,
        surface_ptr: std::ptr::NonNull<std::ffi::c_void>,
    ) -> Option<Self> {
        use wayland_client::backend::{Backend, ObjectId};

        // eframe が使用している既存の wl_display をラップする（所有権は取らない）
        let backend = unsafe { Backend::from_foreign_display(display_ptr.as_ptr() as *mut _) };
        let conn = WlConnection::from_backend(backend);

        let mut state = WaylandDndState {
            last_button_serial: 0,
            seat: None,
            pointer: None,
            data_device_manager: None,
            data_device: None,
            compositor: None,
            shm: None,
            drag_uri: None,
            drag_active: false,
            drag_icon_surface: None,
            current_offer: None,
            offer_has_uri_list: false,
            pending_receive: None,
            received_drops: Vec::new(),
        };

        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();

        // レジストリを取得してグローバルをバインドする
        let _registry = conn.display().get_registry(&qh, ());
        event_queue.roundtrip(&mut state).ok()?;

        // シリアル追跡用のポインタと DnD 用のデータデバイスを作成
        // winit 0.30 は Wayland DnD 受信を未実装のため、自前で data_device を作成して処理する
        if let Some(ref seat) = state.seat {
            if state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(&qh, ()));
            }
            if let Some(ref mgr) = state.data_device_manager {
                if state.data_device.is_none() {
                    state.data_device = Some(mgr.get_data_device(seat, &qh, ()));
                }
            }
        }

        // 追加のイベントを処理するための2回目のラウンドトリップ
        event_queue.roundtrip(&mut state).ok()?;

        // eframe のサーフェスをラップして start_drag の origin として使用可能にする
        let surface_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface_ptr.as_ptr() as *mut _,
            )
        }
        .ok()?;
        let origin_surface = wl_surface::WlSurface::from_id(&conn, surface_id).ok()?;

        let _ = conn.flush();

        Some(WaylandDndContext {
            conn,
            event_queue,
            state,
            origin_surface,
            cursor_theme: None,
        })
    }

    /// 保留中のイベントをディスパッチする
    fn dispatch_pending(&mut self) {
        // まず既にキューにあるイベントを処理
        let _ = self.event_queue.dispatch_pending(&mut self.state);
        // ソケットからの新しいイベントを能動的に読み取る
        // （DnD セッション中は winit がイベントを読み取らない場合があるため）
        let _ = self.conn.flush();
        if let Some(guard) = self.event_queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.event_queue.dispatch_pending(&mut self.state);
        // ディスパッチ中に送出したリクエスト（accept / set_actions / receive など）を
        // 即座にコンポジターへ送る。遅延すると KWin がドロップをタイムアウトさせる。
        let _ = self.conn.flush();
    }

    /// ドロップ受信のパイプからデータを読み取り、URIをパースしてパスに変換する。
    /// 1回の呼び出しでは読めるだけ読み、EOF に達していなければ次フレームに持ち越す
    /// （ドラッグ元が応答しないときに UI スレッドをブロックさせないため）。
    fn complete_pending_drops(&mut self) {
        let Some(mut pending) = self.state.pending_receive.take() else { return };
        // receive リクエストをコンポジターに確実に送信する
        let _ = self.conn.flush();

        use std::io::Read;
        let mut chunk = [0u8; 4096];
        let mut reached_eof = false;
        let mut failed = false;
        loop {
            match pending.stream.read(&mut chunk) {
                Ok(0) => { reached_eof = true; break; }
                Ok(n) => pending.buf.extend_from_slice(&chunk[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                // まだデータが届いていない → 次フレームで続きを読む
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(_) => { failed = true; break; }
            }
        }

        if !reached_eof && !failed {
            if Instant::now() < pending.deadline {
                self.state.pending_receive = Some(pending); // 継続
                return;
            }
            failed = true; // タイムアウト
        }

        let PendingReceive { offer, buf, .. } = pending;

        // データ転送が完了してから finish/destroy を行う（DnD v3 の正しい手順）。
        // KWin/KDE は転送前に finish/destroy するとドロップを中断するため、ここまで遅延させる。
        if offer.version() >= 3 {
            offer.finish();
        }
        offer.destroy();
        let _ = self.conn.flush();

        if !failed {
            let buf = String::from_utf8_lossy(&buf);
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Ok(url) = url::Url::parse(line) {
                    if url.scheme() == "file" {
                        if let Ok(path) = url.to_file_path() {
                            self.state.received_drops.push(path);
                        }
                    }
                }
            }
        }
    }

    /// ドラッグ操作を開始する
    fn start_drag(&mut self, file_path: &Path) {
        if self.state.data_device_manager.is_none() || self.state.data_device.is_none() {
            return;
        }
        if self.state.last_button_serial == 0 {
            eprintln!("Wayland DnD: ボタン押下シリアルが未取得のためドラッグを開始できません");
            return;
        }

        // ファイル URI を生成
        let uri_str = url::Url::from_file_path(file_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", file_path.display()));
        self.state.drag_uri = Some(format!("{}\r\n", uri_str));

        // ドラッグアイコン用のサーフェスを作成（掴んでいる手のカーソル）
        let qh = self.event_queue.handle();
        let icon_surface = self.create_drag_icon_surface(&qh);

        // データソースを作成し、text/uri-list を提供する
        let mgr = self.state.data_device_manager.as_ref().unwrap();
        let source = mgr.create_data_source(&qh, ());
        source.offer("text/uri-list".to_string());
        // サポートするアクションを宣言（これがないとコンポジターがアクション交渉できずドロップが失敗する）
        source.set_actions(wl_data_device_manager::DndAction::Copy | wl_data_device_manager::DndAction::Move);

        // ドラッグを開始
        let device = self.state.data_device.as_ref().unwrap();
        device.start_drag(
            Some(&source),
            &self.origin_surface,
            icon_surface.as_ref(),
            self.state.last_button_serial,
        );

        self.state.drag_icon_surface = icon_surface;
        self.state.drag_active = true;
        let _ = self.conn.flush();
    }

    /// カーソルテーマを初期化する（まだ読み込まれていない場合のみ）
    fn ensure_cursor_theme(&mut self) {
        if self.cursor_theme.is_some() {
            return;
        }
        let Some(ref shm) = self.state.shm else { return };
        let cursor_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(24);
        if let Ok(theme) = wayland_cursor::CursorTheme::load(&self.conn, shm.clone(), cursor_size) {
            self.cursor_theme = Some(theme);
        }
    }

    /// カーソルテーマから "grabbing" カーソルを読み込み、ドラッグアイコン用サーフェスを作成する
    fn create_drag_icon_surface(&mut self, qh: &WlQueueHandle<WaylandDndState>) -> Option<wl_surface::WlSurface> {
        if self.state.compositor.is_none() {
            return None;
        }

        // カーソルテーマを初期化（共有メモリをコンテキストに保持し続ける）
        self.ensure_cursor_theme();
        let cursor_theme = self.cursor_theme.as_mut()?;

        // "grabbing" カーソルを取得（フォールバック: "hand2", "pointer"）
        let cursor = if let Some(c) = cursor_theme.get_cursor("grabbing") {
            c
        } else if let Some(c) = cursor_theme.get_cursor("hand2") {
            c
        } else {
            cursor_theme.get_cursor("pointer")?
        };
        let cursor_image = &cursor[0];
        let (hotspot_x, hotspot_y) = cursor_image.hotspot();

        // アイコン用サーフェスを作成し、カーソル画像をアタッチ
        // attach の x, y パラメータでホットスポット分ずらす（wl_surface v4 互換）
        let compositor = self.state.compositor.as_ref().unwrap();
        let surface = compositor.create_surface(qh, ());
        surface.attach(Some(&**cursor_image), -(hotspot_x as i32), -(hotspot_y as i32));
        surface.commit();

        Some(surface)
    }
}

// --- Wayland Dispatch 実装 ---

// wl_registry: グローバルオブジェクトのバインド
#[cfg(target_os = "linux")]
impl WlDispatch<wl_registry::WlRegistry, ()> for WaylandDndState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _conn: &WlConnection,
        qh: &WlQueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), qh, ());
                    state.seat = Some(seat);
                }
                "wl_data_device_manager" => {
                    let mgr = registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    );
                    state.data_device_manager = Some(mgr);
                }
                "wl_compositor" => {
                    let comp = registry.bind::<wl_compositor::WlCompositor, _, _>(name, version.min(4), qh, ());
                    state.compositor = Some(comp);
                }
                "wl_shm" => {
                    let shm = registry.bind::<wl_shm::WlShm, _, _>(name, version.min(1), qh, ());
                    state.shm = Some(shm);
                }
                _ => {}
            }
        }
    }
}

// wl_seat: ポインタケイパビリティの検出
#[cfg(target_os = "linux")]
impl WlDispatch<wl_seat::WlSeat, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _seat: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
        // ポインタの作成は WaylandDndContext::new() で行う
    }
}

// wl_pointer: ボタン押下イベントのシリアルを追跡（start_drag に必要）
#[cfg(target_os = "linux")]
impl WlDispatch<wl_pointer::WlPointer, ()> for WaylandDndState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
        // start_drag には押下時のシリアルが必要（解放時のシリアルでは無効）
        if let wl_pointer::Event::Button { serial, state: btn_state, .. } = event {
            if btn_state == wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) {
                state.last_button_serial = serial;
            }
        }
    }
}

// wl_data_device_manager: イベントなし
#[cfg(target_os = "linux")]
impl WlDispatch<wl_data_device_manager::WlDataDeviceManager, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _mgr: &wl_data_device_manager::WlDataDeviceManager,
        _event: wl_data_device_manager::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
    }
}

// wl_data_device: ドラッグ＆ドロップイベント（winit が Wayland DnD 受信を未実装のため自前で処理）
#[cfg(target_os = "linux")]
impl WlDispatch<wl_data_device::WlDataDevice, ()> for WaylandDndState {
    fn event(
        state: &mut Self,
        _device: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                // 新しいオファーオブジェクトが作成された。MIME タイプの追跡を開始
                state.offer_has_uri_list = false;
                state.current_offer = Some(id);
            }
            wl_data_device::Event::Enter { serial, id, .. } => {
                // DnD カーソルがサーフェスに入った
                if state.drag_active {
                    return; // 自分自身のドラッグは無視
                }
                if let Some(ref offer) = id {
                    if state.offer_has_uri_list {
                        // text/uri-list を受け入れる
                        offer.accept(serial, Some("text/uri-list".to_string()));
                        offer.set_actions(
                            wl_data_device_manager::DndAction::Copy,
                            wl_data_device_manager::DndAction::Copy,
                        );
                    }
                }
            }
            wl_data_device::Event::Motion { .. } => {
                // ドラッグ中のカーソル移動（特に処理不要）
            }
            wl_data_device::Event::Drop => {
                // ユーザーがドロップした → パイプ経由でデータを受信する
                if let Some(offer) = state.current_offer.take() {
                    // 自分が始めたドラッグを自分のウィンドウに落とした場合は受信しない
                    // （Enter と同じガード。これがないと自分自身を読み直してしまう）
                    if state.drag_active {
                        offer.destroy();
                    } else if state.offer_has_uri_list {
                        if let Ok((reader, writer)) = std::os::unix::net::UnixStream::pair() {
                            use std::os::fd::AsFd;
                            offer.receive("text/uri-list".to_string(), writer.as_fd());
                            drop(writer); // 書き込み側を閉じて EOF を発生させる
                            // UI スレッドから読むためノンブロッキングにする
                            let _ = reader.set_nonblocking(true);
                            // finish()/destroy() はデータ読み取り完了後（complete_pending_drops）に行う。
                            // 転送前に呼ぶと KWin/KDE がドロップを中断するため offer を持ち越す。
                            state.pending_receive = Some(PendingReceive {
                                stream: reader,
                                offer,
                                buf: Vec::new(),
                                deadline: Instant::now() + DROP_RECEIVE_TIMEOUT,
                            });
                        } else {
                            offer.destroy();
                        }
                    } else {
                        offer.destroy();
                    }
                }
                state.offer_has_uri_list = false;
            }
            wl_data_device::Event::Leave => {
                // DnD カーソルがサーフェスから離れた（キャンセル）
                if let Some(ref offer) = state.current_offer {
                    offer.destroy();
                }
                state.current_offer = None;
                state.offer_has_uri_list = false;
            }
            _ => {}
        }
    }

    // data_offer イベント (opcode 0) が新しい WlDataOffer オブジェクトを作成するため必要
    wayland_client::event_created_child!(WaylandDndState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

// wl_data_source: ドラッグソースのイベント処理
#[cfg(target_os = "linux")]
impl WlDispatch<wl_data_source::WlDataSource, ()> for WaylandDndState {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                // ドロップターゲットがデータを要求した → URI を fd に書き込む
                if mime_type == "text/uri-list" {
                    if let Some(ref uri) = state.drag_uri {
                        use std::io::Write;
                        let mut file = std::fs::File::from(fd);
                        let _ = file.write_all(uri.as_bytes());
                    }
                }
            }
            wl_data_source::Event::DndDropPerformed => {
                // ドロップ操作そのものは完了した。
                // dnd_finished は wl_data_device_manager v3 以上かつ受け手が finish() を
                // 呼んだ場合にしか届かないため、ここでもドラッグ状態を解除しておかないと
                // drag_active が true のまま固着し、以後の受信ドロップが全て無視される。
                state.drag_active = false;
                if let Some(icon) = state.drag_icon_surface.take() {
                    icon.destroy();
                }
                // データ転送要求（Send）がこの後に来る可能性があるため
                // drag_uri と source はここでは破棄しない
            }
            wl_data_source::Event::DndFinished => {
                // ドロップ完了
                state.drag_active = false;
                state.drag_uri = None;
                source.destroy();
                if let Some(icon) = state.drag_icon_surface.take() {
                    icon.destroy();
                }
            }
            wl_data_source::Event::Cancelled => {
                // ドラッグキャンセル
                state.drag_active = false;
                state.drag_uri = None;
                source.destroy();
                if let Some(icon) = state.drag_icon_surface.take() {
                    icon.destroy();
                }
            }
            _ => {}
        }
    }
}

// wl_data_offer: MIME タイプの追跡
#[cfg(target_os = "linux")]
impl WlDispatch<wl_data_offer::WlDataOffer, ()> for WaylandDndState {
    fn event(
        state: &mut Self,
        _offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if mime_type == "text/uri-list" {
                state.offer_has_uri_list = true;
            }
        }
    }
}

// wl_surface: eframe のサーフェスおよびドラッグアイコンサーフェス用
#[cfg(target_os = "linux")]
impl WlDispatch<wl_surface::WlSurface, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _surface: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
    }
}

// wl_compositor: イベントなし（ドラッグアイコン用サーフェス作成に使用）
#[cfg(target_os = "linux")]
impl WlDispatch<wl_compositor::WlCompositor, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _compositor: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
    }
}

// wl_shm: イベントなし（カーソルテーマ読み込みに使用）
#[cfg(target_os = "linux")]
impl WlDispatch<wl_shm::WlShm, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _shm: &wl_shm::WlShm,
        _event: wl_shm::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
    }
}

// wl_buffer: カーソル画像バッファ用
#[cfg(target_os = "linux")]
impl WlDispatch<wl_buffer::WlBuffer, ()> for WaylandDndState {
    fn event(
        _state: &mut Self,
        _buffer: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _: &(),
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
    ) {
    }
}

// 文字イテレータから連続する数字を読み取って数字列のまま返す。
// u64 にパースすると 20 桁を超える数字列が全て u64::MAX に飽和して
// 区別できなくなるため、文字列として返して呼び出し側で桁数比較させる。
fn collect_digits(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut digits = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            digits.push(c);
            chars.next(); // 次の文字へ
        } else {
            break; // 数字でなくなったらループを抜ける
        }
    }
    digits
}
