//! 表示中のフォルダの画像を、ファイル名と解像度で絞り込む条件。

use std::path::Path;

/// フィルターウインドウの入力値。数値欄は空なら制限なしとして扱う
#[derive(Default)]
pub struct FilterInput {
    pub name: String,  // ファイル名（空白区切りで AND、`*` / `?` を含めばワイルドカード）
    pub min_w: String, // 幅の下限
    pub max_w: String, // 幅の上限
    pub min_h: String, // 高さの下限
    pub max_h: String, // 高さの上限
}

/// 入力値を解釈した絞り込み条件
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    terms: Vec<String>, // 小文字にしたファイル名の条件
    min_w: Option<u32>,
    max_w: Option<u32>,
    min_h: Option<u32>,
    max_h: Option<u32>,
}

impl FilterInput {
    /// 入力値を条件に直す。数値として読めない欄は `Err` にその欄の名前を返す
    pub fn parse(&self) -> Result<Filter, Vec<&'static str>> {
        let mut invalid = Vec::new();
        let mut num = |s: &str, label: &'static str| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            match s.parse::<u32>() {
                Ok(v) => Some(v),
                Err(_) => {
                    invalid.push(label);
                    None
                }
            }
        };
        let filter = Filter {
            terms: self.name.split_whitespace().map(str::to_lowercase).collect(),
            min_w: num(&self.min_w, "幅の下限"),
            max_w: num(&self.max_w, "幅の上限"),
            min_h: num(&self.min_h, "高さの下限"),
            max_h: num(&self.max_h, "高さの上限"),
        };
        if invalid.is_empty() { Ok(filter) } else { Err(invalid) }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl Filter {
    /// 何か条件が入っているか
    pub fn is_active(&self) -> bool {
        !self.terms.is_empty() || self.needs_dimensions()
    }

    /// 判定に画像の解像度が必要か
    pub fn needs_dimensions(&self) -> bool {
        self.min_w.is_some() || self.max_w.is_some() || self.min_h.is_some() || self.max_h.is_some()
    }

    /// ファイル名の条件に合うか（大文字小文字は区別しない）
    pub fn matches_name(&self, path: &Path) -> bool {
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        self.terms.iter().all(|t| {
            if t.contains(['*', '?']) {
                wildcard_match(t, &name)
            } else {
                name.contains(t.as_str())
            }
        })
    }

    /// 解像度の条件に合うか。`None`（未取得・読めない画像）は解像度の条件があれば合わないとする
    pub fn matches_dimensions(&self, dims: Option<(u32, u32)>) -> bool {
        if !self.needs_dimensions() {
            return true;
        }
        let Some((w, h)) = dims else { return false };
        let within = |v: u32, min: Option<u32>, max: Option<u32>| {
            min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m)
        };
        within(w, self.min_w, self.max_w) && within(h, self.min_h, self.max_h)
    }
}

/// `*`（0 文字以上）と `?`（1 文字）のワイルドカードで、名前全体が一致するか
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    // 直前の `*` の位置と、そのとき合わせ始めた text の位置（戻り先）
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            // `*` に 1 文字多く吸わせてやり直す
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, min_w: &str, max_w: &str, min_h: &str, max_h: &str) -> FilterInput {
        FilterInput {
            name: name.into(),
            min_w: min_w.into(),
            max_w: max_w.into(),
            min_h: min_h.into(),
            max_h: max_h.into(),
        }
    }

    #[test]
    fn wildcard() {
        assert!(wildcard_match("*.png", "a.png"));
        assert!(!wildcard_match("*.png", "a.png.jpg"));
        assert!(wildcard_match("img_??.jpg", "img_01.jpg"));
        assert!(!wildcard_match("img_??.jpg", "img_1.jpg"));
        assert!(wildcard_match("*cat*", "my_cat_01.jpg"));
        assert!(wildcard_match("a*b*c", "aXXbYYc"));
        assert!(!wildcard_match("a*b*c", "aXXbYY"));
        assert!(wildcard_match("日本*", "日本語.png"));
    }

    #[test]
    fn name_terms_are_and_and_case_insensitive() {
        let f = input("Cat  2024", "", "", "", "").parse().unwrap();
        assert!(f.matches_name(Path::new("/x/MyCAT_2024.jpg")));
        assert!(!f.matches_name(Path::new("/x/cat_2023.jpg")));
        let f = input("*.PNG", "", "", "", "").parse().unwrap();
        assert!(f.matches_name(Path::new("a.png")));
        assert!(!f.matches_name(Path::new("a.jpg")));
    }

    #[test]
    fn dimensions() {
        let f = input("", "1000", "", "", "800").parse().unwrap();
        assert!(f.is_active() && f.needs_dimensions());
        assert!(f.matches_dimensions(Some((1000, 800))));
        assert!(!f.matches_dimensions(Some((999, 600))));
        assert!(!f.matches_dimensions(Some((1920, 1080))));
        assert!(!f.matches_dimensions(None));

        let none = input(" ", "", "", "", "").parse().unwrap();
        assert!(!none.is_active());
        assert!(none.matches_dimensions(None));
    }

    #[test]
    fn invalid_numbers_are_reported() {
        assert_eq!(input("", "abc", "", "", "-1").parse(), Err(vec!["幅の下限", "高さの上限"]));
    }
}
