//! RTF rich-text parser, multi-run word wrapper, and card text renderer.
//! Ensures formatting (bold, italic, underline, strikeout, fonts, sizes, colors)
//! and lists (bullets/numbering) saved in Note::rtf persist when exiting input mode.
use super::*;
use std::collections::HashMap;

#[link(name = "kernel32")]
extern "system" {
    fn MultiByteToWideChar(
        code_page: u32,
        flags: u32,
        src: *const u8,
        src_len: i32,
        dst: *mut u16,
        dst_len: i32,
    ) -> i32;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub family: Option<String>,
    pub size_dip: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<u32>,
}

impl Run {
    fn same_style(&self, other: &Self) -> bool {
        self.family == other.family
            && self.size_dip == other.size_dip
            && self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.strike == other.strike
            && self.color == other.color
    }

    pub fn style_bits(&self) -> u8 {
        u8::from(self.bold)
            | (u8::from(self.italic) << 1)
            | (u8::from(self.underline) << 2)
            | (u8::from(self.strike) << 3)
    }

    fn effective_size(&self, base_body: f32) -> f32 {
        self.size_dip.unwrap_or(base_body)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub prefix: Option<String>,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug)]
pub struct RichLine {
    pub indent: f32,
    pub runs: Vec<Run>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Normal,
    FontTbl,
    ColorTbl,
    Pn,
    PnText,
    Ignore,
}

#[derive(Clone)]
struct GroupState {
    dest: Dest,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    font_idx: i32,
    half_pts: i32,
    color_idx: usize,
    uc: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Numbered,
}

fn cp1252_char(byte: u8) -> char {
    match byte {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        _ => byte as char,
    }
}

fn is_default_font_name(name: &str) -> bool {
    let t = name.trim();
    t.is_empty()
        || t.eq_ignore_ascii_case("Segoe UI Variable Text")
        || t.eq_ignore_ascii_case("Segoe UI")
}

fn is_symbol_font_name(name: &str) -> bool {
    let t = name.trim();
    t.eq_ignore_ascii_case("Symbol")
        || t.eq_ignore_ascii_case("Wingdings")
        || t.eq_ignore_ascii_case("Webdings")
}

fn normalize_pntext(raw: &str, list_kind: Option<ListKind>) -> String {
    let mut cleaned = String::with_capacity(raw.len() + 2);
    for ch in raw.chars() {
        match ch {
            '\t' | '\r' | '\n' => {
                if !cleaned.ends_with(' ') && !cleaned.is_empty() {
                    cleaned.push(' ');
                }
            }
            '\u{00B7}' if list_kind != Some(ListKind::Numbered) => cleaned.push('•'),
            _ => cleaned.push(ch),
        }
    }
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "·" || trimmed == "\u{F0B7}" {
        "• ".into()
    } else {
        format!("{trimmed} ")
    }
}

fn charset_to_codepage(cs: i32) -> Option<u32> {
    match cs {
        0 => Some(1252),
        1 => None,
        163 => Some(1258),
        238 => Some(1250),
        204 => Some(1251),
        161 => Some(1253),
        162 => Some(1254),
        177 => Some(1255),
        178 => Some(1256),
        186 => Some(1257),
        222 => Some(874),
        128 => Some(932),
        134 => Some(936),
        129 => Some(949),
        136 => Some(950),
        _ => None,
    }
}

fn compose_vietnamese(base: char, mark: char) -> Option<char> {
    match (base, mark) {
        ('a', '\u{0300}') => Some('à'),
        ('a', '\u{0301}') => Some('á'),
        ('a', '\u{0303}') => Some('ã'),
        ('a', '\u{0309}') => Some('ả'),
        ('a', '\u{0323}') => Some('ạ'),
        ('A', '\u{0300}') => Some('À'),
        ('A', '\u{0301}') => Some('Á'),
        ('A', '\u{0303}') => Some('Ã'),
        ('A', '\u{0309}') => Some('Ả'),
        ('A', '\u{0323}') => Some('Ạ'),
        ('ă', '\u{0300}') => Some('ằ'),
        ('ă', '\u{0301}') => Some('ắ'),
        ('ă', '\u{0303}') => Some('ẵ'),
        ('ă', '\u{0309}') => Some('ẳ'),
        ('ă', '\u{0323}') => Some('ặ'),
        ('Ă', '\u{0300}') => Some('Ằ'),
        ('Ă', '\u{0301}') => Some('Ắ'),
        ('Ă', '\u{0303}') => Some('Ẵ'),
        ('Ă', '\u{0309}') => Some('Ẳ'),
        ('Ă', '\u{0323}') => Some('Ặ'),
        ('â', '\u{0300}') => Some('ầ'),
        ('â', '\u{0301}') => Some('ấ'),
        ('â', '\u{0303}') => Some('ẫ'),
        ('â', '\u{0309}') => Some('ẩ'),
        ('â', '\u{0323}') => Some('ậ'),
        ('Â', '\u{0300}') => Some('Ầ'),
        ('Â', '\u{0301}') => Some('Ấ'),
        ('Â', '\u{0303}') => Some('Ẫ'),
        ('Â', '\u{0309}') => Some('Ẩ'),
        ('Â', '\u{0323}') => Some('Ậ'),
        ('e', '\u{0300}') => Some('è'),
        ('e', '\u{0301}') => Some('é'),
        ('e', '\u{0303}') => Some('ẽ'),
        ('e', '\u{0309}') => Some('ẻ'),
        ('e', '\u{0323}') => Some('ẹ'),
        ('E', '\u{0300}') => Some('È'),
        ('E', '\u{0301}') => Some('É'),
        ('E', '\u{0303}') => Some('Ẽ'),
        ('E', '\u{0309}') => Some('Ẻ'),
        ('E', '\u{0323}') => Some('Ẹ'),
        ('ê', '\u{0300}') => Some('ề'),
        ('ê', '\u{0301}') => Some('ế'),
        ('ê', '\u{0303}') => Some('ễ'),
        ('ê', '\u{0309}') => Some('ể'),
        ('ê', '\u{0323}') => Some('ệ'),
        ('Ê', '\u{0300}') => Some('Ề'),
        ('Ê', '\u{0301}') => Some('Ế'),
        ('Ê', '\u{0303}') => Some('Ễ'),
        ('Ê', '\u{0309}') => Some('Ể'),
        ('Ê', '\u{0323}') => Some('Ệ'),
        ('i', '\u{0300}') => Some('ì'),
        ('i', '\u{0301}') => Some('í'),
        ('i', '\u{0303}') => Some('ĩ'),
        ('i', '\u{0309}') => Some('ỉ'),
        ('i', '\u{0323}') => Some('ị'),
        ('I', '\u{0300}') => Some('Ì'),
        ('I', '\u{0301}') => Some('Í'),
        ('I', '\u{0303}') => Some('Ĩ'),
        ('I', '\u{0309}') => Some('Ỉ'),
        ('I', '\u{0323}') => Some('Ị'),
        ('o', '\u{0300}') => Some('ò'),
        ('o', '\u{0301}') => Some('ó'),
        ('o', '\u{0303}') => Some('õ'),
        ('o', '\u{0309}') => Some('ỏ'),
        ('o', '\u{0323}') => Some('ọ'),
        ('O', '\u{0300}') => Some('Ò'),
        ('O', '\u{0301}') => Some('Ó'),
        ('O', '\u{0303}') => Some('Õ'),
        ('O', '\u{0309}') => Some('Ỏ'),
        ('O', '\u{0323}') => Some('Ọ'),
        ('ô', '\u{0300}') => Some('ồ'),
        ('ô', '\u{0301}') => Some('ố'),
        ('ô', '\u{0303}') => Some('ỗ'),
        ('ô', '\u{0309}') => Some('ổ'),
        ('ô', '\u{0323}') => Some('ộ'),
        ('Ô', '\u{0300}') => Some('Ồ'),
        ('Ô', '\u{0301}') => Some('Ố'),
        ('Ô', '\u{0303}') => Some('Ỗ'),
        ('Ô', '\u{0309}') => Some('Ổ'),
        ('Ô', '\u{0323}') => Some('Ộ'),
        ('ơ', '\u{0300}') => Some('ờ'),
        ('ơ', '\u{0301}') => Some('ớ'),
        ('ơ', '\u{0303}') => Some('ỡ'),
        ('ơ', '\u{0309}') => Some('ở'),
        ('ơ', '\u{0323}') => Some('ợ'),
        ('Ơ', '\u{0300}') => Some('Ờ'),
        ('Ơ', '\u{0301}') => Some('Ớ'),
        ('Ơ', '\u{0303}') => Some('Ỡ'),
        ('Ơ', '\u{0309}') => Some('Ở'),
        ('Ơ', '\u{0323}') => Some('Ợ'),
        ('u', '\u{0300}') => Some('ù'),
        ('u', '\u{0301}') => Some('ú'),
        ('u', '\u{0303}') => Some('ũ'),
        ('u', '\u{0309}') => Some('ủ'),
        ('u', '\u{0323}') => Some('ụ'),
        ('U', '\u{0300}') => Some('Ù'),
        ('U', '\u{0301}') => Some('Ú'),
        ('U', '\u{0303}') => Some('Ũ'),
        ('U', '\u{0309}') => Some('Ủ'),
        ('U', '\u{0323}') => Some('Ụ'),
        ('ư', '\u{0300}') => Some('ừ'),
        ('ư', '\u{0301}') => Some('ứ'),
        ('ư', '\u{0303}') => Some('ữ'),
        ('ư', '\u{0309}') => Some('ử'),
        ('ư', '\u{0323}') => Some('ự'),
        ('Ư', '\u{0300}') => Some('Ừ'),
        ('Ư', '\u{0301}') => Some('Ứ'),
        ('Ư', '\u{0303}') => Some('Ữ'),
        ('Ư', '\u{0309}') => Some('Ử'),
        ('Ư', '\u{0323}') => Some('Ự'),
        ('y', '\u{0300}') => Some('ỳ'),
        ('y', '\u{0301}') => Some('ý'),
        ('y', '\u{0303}') => Some('ỹ'),
        ('y', '\u{0309}') => Some('ỷ'),
        ('y', '\u{0323}') => Some('ỵ'),
        ('Y', '\u{0300}') => Some('Ỳ'),
        ('Y', '\u{0301}') => Some('Ý'),
        ('Y', '\u{0303}') => Some('Ỹ'),
        ('Y', '\u{0309}') => Some('Ỷ'),
        ('Y', '\u{0323}') => Some('Ỵ'),
        _ => None,
    }
}

struct RtfParser {
    base_body_size: f32,
    doc_codepage: u32,
    fonttbl: HashMap<i32, String>,
    font_charsets: HashMap<i32, i32>,
    cur_font_idx: Option<i32>,
    cur_font_charset: Option<i32>,
    cur_font_name: String,
    colortbl: Vec<Option<u32>>,
    ct_r: Option<u8>,
    ct_g: Option<u8>,
    ct_b: Option<u8>,
    ct_has_rgb: bool,
    cur_runs: Vec<Run>,
    cur_text: String,
    cur_style_snap: Option<Run>,
    pntext_buf: String,
    utf16_buf: Vec<u16>,
    ansi_buf: Vec<u8>,
    ansi_cp: u32,
}

impl RtfParser {
    fn new(base_body_size: f32) -> Self {
        Self {
            base_body_size,
            doc_codepage: 1252,
            fonttbl: HashMap::new(),
            font_charsets: HashMap::new(),
            cur_font_idx: None,
            cur_font_charset: None,
            cur_font_name: String::new(),
            colortbl: Vec::new(),
            ct_r: None,
            ct_g: None,
            ct_b: None,
            ct_has_rgb: false,
            cur_runs: Vec::new(),
            cur_text: String::new(),
            cur_style_snap: None,
            pntext_buf: String::new(),
            utf16_buf: Vec::new(),
            ansi_buf: Vec::new(),
            ansi_cp: 1252,
        }
    }

    fn active_codepage(&self, s: &GroupState) -> u32 {
        self.font_charsets
            .get(&s.font_idx)
            .and_then(|&cs| charset_to_codepage(cs))
            .unwrap_or(self.doc_codepage)
    }

    fn commit_font(&mut self) {
        if let Some(idx) = self.cur_font_idx.take() {
            let name = self.cur_font_name.trim().trim_end_matches(';').trim();
            if !name.is_empty() {
                self.fonttbl.insert(idx, name.to_string());
            }
            if let Some(cs) = self.cur_font_charset.take() {
                self.font_charsets.insert(idx, cs);
            }
        } else {
            self.cur_font_charset = None;
        }
        self.cur_font_name.clear();
    }

    fn make_run(&self, text: String, s: &GroupState) -> Run {
        let family = self.fonttbl.get(&s.font_idx).and_then(|name| {
            if is_default_font_name(name) || is_symbol_font_name(name) {
                None
            } else {
                Some(name.clone())
            }
        });
        let size_dip = if s.half_pts > 0
            && (s.half_pts as f32 - self.base_body_size * 1.5).abs() > 1.25
        {
            Some((s.half_pts as f32 * (2.0 / 3.0)).clamp(9.0, 36.0))
        } else {
            None
        };
        let color = self.colortbl.get(s.color_idx).copied().flatten();
        Run {
            text,
            family,
            size_dip,
            bold: s.bold,
            italic: s.italic,
            underline: s.underline,
            strike: s.strike,
            color,
        }
    }

    fn flush_run(&mut self, s: &GroupState) {
        if self.cur_text.is_empty() {
            return;
        }
        let t = std::mem::take(&mut self.cur_text);
        let mut new_run = self
            .cur_style_snap
            .take()
            .unwrap_or_else(|| self.make_run(String::new(), s));
        new_run.text = t;
        if let Some(last) = self.cur_runs.last_mut() {
            if last.same_style(&new_run) {
                last.text.push_str(&new_run.text);
                return;
            }
        }
        self.cur_runs.push(new_run);
    }

    fn push_char(&mut self, ch: char, s: &GroupState) {
        match s.dest {
            Dest::Normal => {
                let desired = self.make_run(String::new(), s);
                if self
                    .cur_style_snap
                    .as_ref()
                    .is_some_and(|prev| !prev.same_style(&desired))
                {
                    self.flush_run(s);
                }
                if self.cur_style_snap.is_none() {
                    self.cur_style_snap = Some(desired);
                }
                if matches!(ch, '\u{0300}' | '\u{0301}' | '\u{0303}' | '\u{0309}' | '\u{0323}') {
                    if let Some(base) = self.cur_text.chars().last() {
                        if let Some(composed) = compose_vietnamese(base, ch) {
                            self.cur_text.pop();
                            self.cur_text.push(composed);
                            return;
                        }
                    }
                }
                self.cur_text.push(ch);
            }
            Dest::FontTbl => {
                if ch == ';' {
                    self.commit_font();
                } else {
                    self.cur_font_name.push(ch);
                }
            }
            Dest::ColorTbl => {
                if ch == ';' {
                    let color = if self.ct_has_rgb {
                        let r = self.ct_r.unwrap_or(0);
                        let g = self.ct_g.unwrap_or(0);
                        let b = self.ct_b.unwrap_or(0);
                        // Auto/black in RichEdit RTF should adopt adaptive card ink.
                        if r < 24 && g < 24 && b < 24 {
                            None
                        } else {
                            Some(super::rgb(r, g, b))
                        }
                    } else {
                        None
                    };
                    self.colortbl.push(color);
                    self.ct_r = None;
                    self.ct_g = None;
                    self.ct_b = None;
                    self.ct_has_rgb = false;
                }
            }
            Dest::PnText => self.pntext_buf.push(ch),
            Dest::Pn | Dest::Ignore => {}
        }
    }

    fn flush_utf16(&mut self, s: &GroupState) {
        if self.utf16_buf.is_empty() {
            return;
        }
        let mut units = std::mem::take(&mut self.utf16_buf);
        for decoded in char::decode_utf16(units.drain(..)) {
            let ch = decoded.unwrap_or('\u{FFFD}');
            match s.dest {
                Dest::Normal | Dest::PnText => self.push_char(ch, s),
                _ => {}
            }
        }
        self.utf16_buf = units;
    }

    fn flush_ansi(&mut self, s: &GroupState) {
        if self.ansi_buf.is_empty() {
            return;
        }
        let mut bytes = std::mem::take(&mut self.ansi_buf);
        let needed = unsafe {
            MultiByteToWideChar(
                self.ansi_cp,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                std::ptr::null_mut(),
                0,
            )
        };
        let mut decoded_ok = false;
        if needed > 0 {
            let mut wbuf = vec![0u16; needed as usize];
            let written = unsafe {
                MultiByteToWideChar(
                    self.ansi_cp,
                    0,
                    bytes.as_ptr(),
                    bytes.len() as i32,
                    wbuf.as_mut_ptr(),
                    needed,
                )
            };
            if written > 0 {
                decoded_ok = true;
                for decoded in char::decode_utf16(wbuf[..written as usize].iter().copied()) {
                    self.push_char(decoded.unwrap_or('\u{FFFD}'), s);
                }
            }
        }
        if !decoded_ok {
            for &b in &bytes {
                self.push_char(cp1252_char(b), s);
            }
        }
        bytes.clear();
        self.ansi_buf = bytes;
    }
}

pub fn parse_rtf(rtf: &str, base_body_size: f32) -> Vec<Paragraph> {
    if !rtf.trim_start().starts_with('{') {
        return rtf
            .lines()
            .map(|line| Paragraph {
                prefix: None,
                runs: if line.is_empty() {
                    vec![]
                } else {
                    vec![Run {
                        text: line.into(),
                        family: None,
                        size_dip: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        strike: false,
                        color: None,
                    }]
                },
            })
            .collect();
    }

    let bytes = rtf.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<GroupState> = Vec::new();
    let mut st = GroupState {
        dest: Dest::Normal,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        font_idx: 0,
        half_pts: 0,
        color_idx: 0,
        uc: 1,
    };

    let mut p = RtfParser::new(base_body_size);
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut cur_prefix: Option<String> = None;
    let mut list_kind: Option<ListKind> = None;
    let mut pn_start: usize = 1;
    let mut list_counter: usize = 0;
    let mut prev_list_kind: Option<ListKind> = None;
    let mut pntext_symbol_font = false;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => {
                p.flush_ansi(&st);
                p.flush_utf16(&st);
                if st.dest == Dest::Normal {
                    p.flush_run(&st);
                }
                stack.push(st.clone());
                if st.dest == Dest::PnText {
                    p.pntext_buf.clear();
                }
                i += 1;
            }
            b'}' => {
                p.flush_ansi(&st);
                p.flush_utf16(&st);
                match st.dest {
                    Dest::Normal => p.flush_run(&st),
                    Dest::FontTbl => p.commit_font(),
                    Dest::PnText => {
                        let prefix = normalize_pntext(&p.pntext_buf, list_kind);
                        if !prefix.is_empty() {
                            if cur_prefix.is_none() {
                                cur_prefix = Some(prefix);
                            }
                        } else if pntext_symbol_font
                            && !p.pntext_buf.is_empty()
                            && cur_prefix.is_none()
                        {
                            cur_prefix = Some("• ".into());
                        }
                        p.pntext_buf.clear();
                        pntext_symbol_font = false;
                    }
                    _ => {}
                }
                if let Some(prev) = stack.pop() {
                    st = prev;
                }
                i += 1;
            }
            b'\r' | b'\n' => {
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                let next = bytes[i];
                if !next.is_ascii_alphabetic() {
                    if next != b'\'' {
                        p.flush_ansi(&st);
                        p.flush_utf16(&st);
                    }
                    match next {
                        b'*' => {
                            i += 1;
                            // Peek whether this ignorable group is {\*\pn ...}
                            let rest = &bytes[i..];
                            let mut k = 0;
                            while k < rest.len() && rest[k] == b' ' {
                                k += 1;
                            }
                            if rest[k..].starts_with(b"\\pn")
                                && rest.get(k + 3).is_some_and(|c| !c.is_ascii_alphabetic())
                            {
                                st.dest = Dest::Pn;
                            } else {
                                st.dest = Dest::Ignore;
                            }
                        }
                        b'\'' => {
                            i += 1;
                            if i + 1 < bytes.len() {
                                if let Ok(hex_str) = std::str::from_utf8(&bytes[i..i + 2]) {
                                    if let Ok(byte_val) = u8::from_str_radix(hex_str, 16) {
                                        i += 2;
                                        if st.dest == Dest::PnText
                                            && (pntext_symbol_font || byte_val == 0xB7)
                                            && list_kind != Some(ListKind::Numbered)
                                        {
                                            p.flush_ansi(&st);
                                            p.flush_utf16(&st);
                                            p.push_char('•', &st);
                                        } else {
                                            p.flush_utf16(&st);
                                            let cp = p.active_codepage(&st);
                                            if !p.ansi_buf.is_empty() && p.ansi_cp != cp {
                                                p.flush_ansi(&st);
                                            }
                                            p.ansi_cp = cp;
                                            p.ansi_buf.push(byte_val);
                                        }
                                        continue;
                                    }
                                }
                            }
                            p.flush_ansi(&st);
                            p.flush_utf16(&st);
                            i += 1;
                        }
                        b'\\' | b'{' | b'}' => {
                            i += 1;
                            p.push_char(next as char, &st);
                        }
                        b'~' => {
                            i += 1;
                            p.push_char(' ', &st);
                        }
                        b'_' => {
                            i += 1;
                            p.push_char('-', &st);
                        }
                        b'-' => {
                            i += 1;
                        }
                        b'\r' | b'\n' => {
                            i += 1;
                            if st.dest == Dest::Normal {
                                p.flush_run(&st);
                                paragraphs.push(finalize_paragraph(
                                    &mut cur_prefix,
                                    &mut list_kind,
                                    &mut prev_list_kind,
                                    &mut list_counter,
                                    pn_start,
                                    &mut p.cur_runs,
                                ));
                                pn_start = 1;
                            }
                        }
                        _ => {
                            i += 1;
                        }
                    }
                    continue;
                }

                p.flush_ansi(&st);

                let word_start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &rtf[word_start..i];
                let mut param: Option<i32> = None;
                if i < bytes.len() && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                    let p_start = i;
                    if bytes[i] == b'-' {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if let Ok(v) = rtf[p_start..i].parse::<i32>() {
                        param = Some(v);
                    }
                }
                if i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }

                if word == "u" {
                    if let Some(v) = param {
                        let unit = v as i16 as u16;
                        p.utf16_buf.push(unit);
                        // Skip \uc replacement ANSI characters (including \'hh hex escape as 1 char).
                        let mut skip = st.uc;
                        while skip > 0 && i < bytes.len() {
                            if bytes[i] == b'{' || bytes[i] == b'}' {
                                break;
                            }
                            if bytes[i] == b'\\' {
                                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                                    i = (i + 4).min(bytes.len());
                                    skip -= 1;
                                    continue;
                                }
                                break;
                            }
                            if bytes[i] == b'\r' || bytes[i] == b'\n' {
                                i += 1;
                                continue;
                            }
                            i += 1;
                            skip -= 1;
                        }
                        continue;
                    }
                }

                p.flush_utf16(&st);

                match word {
                    "ansicpg" => {
                        if let Some(v) = param.filter(|v| *v > 0) {
                            p.doc_codepage = v as u32;
                        }
                    }
                    "fonttbl" => st.dest = Dest::FontTbl,
                    "colortbl" => st.dest = Dest::ColorTbl,
                    "stylesheet" | "info" | "pict" | "object" | "header" | "footer"
                    | "headerl" | "headerr" | "headerf" | "footerl" | "footerr" | "footerf"
                    | "listtable" | "listoverridetable" => st.dest = Dest::Ignore,
                    "pntext" | "listtext" if st.dest == Dest::Normal => {
                        st.dest = Dest::PnText;
                        p.pntext_buf.clear();
                        pntext_symbol_font = false;
                    }
                    "f" => {
                        if let Some(idx) = param {
                            match st.dest {
                                Dest::FontTbl => {
                                    p.cur_font_idx = Some(idx);
                                    p.cur_font_name.clear();
                                    p.cur_font_charset = None;
                                }
                                Dest::PnText => {
                                    if p.fonttbl
                                        .get(&idx)
                                        .is_some_and(|n| is_symbol_font_name(n))
                                    {
                                        pntext_symbol_font = true;
                                    }
                                }
                                Dest::Normal => {
                                    if p.cur_text.is_empty() {
                                        st.font_idx = idx;
                                    } else {
                                        p.flush_run(&st);
                                        st.font_idx = idx;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "fcharset" if st.dest == Dest::FontTbl => {
                        p.cur_font_charset = param;
                    }
                    "red" if st.dest == Dest::ColorTbl => {
                        p.ct_r = param.map(|v| v.clamp(0, 255) as u8);
                        p.ct_has_rgb = true;
                    }
                    "green" if st.dest == Dest::ColorTbl => {
                        p.ct_g = param.map(|v| v.clamp(0, 255) as u8);
                        p.ct_has_rgb = true;
                    }
                    "blue" if st.dest == Dest::ColorTbl => {
                        p.ct_b = param.map(|v| v.clamp(0, 255) as u8);
                        p.ct_has_rgb = true;
                    }
                    "pnlvlblt" if st.dest == Dest::Pn => list_kind = Some(ListKind::Bullet),
                    "pnlvlbody" | "pnlvl1" | "pnlvl2" | "pnlvl3" | "pnlvl4"
                        if st.dest == Dest::Pn =>
                    {
                        list_kind = Some(ListKind::Numbered);
                    }
                    "pnstart" if st.dest == Dest::Pn => {
                        if let Some(v) = param.filter(|v| *v > 0) {
                            pn_start = v as usize;
                        }
                    }
                    _ if st.dest == Dest::FontTbl || st.dest == Dest::ColorTbl
                        || st.dest == Dest::Ignore || st.dest == Dest::Pn => {}
                    "uc" => {
                        if let Some(v) = param {
                            st.uc = v.max(0) as usize;
                        }
                    }
                    "pard" if st.dest == Dest::Normal => {
                        cur_prefix = None;
                        list_kind = None;
                    }
                    "plain" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.bold = false;
                        st.italic = false;
                        st.underline = false;
                        st.strike = false;
                        st.font_idx = 0;
                        st.half_pts = 0;
                        st.color_idx = 0;
                    }
                    "b" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.bold = param != Some(0);
                    }
                    "i" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.italic = param != Some(0);
                    }
                    "ul" | "uld" | "uldb" | "ulth" | "ulwave" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.underline = param != Some(0);
                    }
                    "ulnone" | "ul0" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.underline = false;
                    }
                    "strike" | "striked" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        st.strike = param != Some(0);
                    }
                    "fs" if st.dest == Dest::Normal => {
                        if let Some(v) = param {
                            p.flush_run(&st);
                            st.half_pts = v;
                        }
                    }
                    "cf" if st.dest == Dest::Normal => {
                        if let Some(v) = param.filter(|v| *v >= 0) {
                            p.flush_run(&st);
                            st.color_idx = v as usize;
                        }
                    }
                    "par" if st.dest == Dest::Normal => {
                        p.flush_run(&st);
                        paragraphs.push(finalize_paragraph(
                            &mut cur_prefix,
                            &mut list_kind,
                            &mut prev_list_kind,
                            &mut list_counter,
                            pn_start,
                            &mut p.cur_runs,
                        ));
                        pn_start = 1;
                    }
                    "line" if st.dest == Dest::Normal => p.push_char('\n', &st),
                    "tab" => p.push_char(if st.dest == Dest::PnText { ' ' } else { '\t' }, &st),
                    "bullet" => p.push_char('•', &st),
                    "emdash" => p.push_char('—', &st),
                    "endash" => p.push_char('–', &st),
                    "lquote" => p.push_char('‘', &st),
                    "rquote" => p.push_char('’', &st),
                    "ldblquote" => p.push_char('“', &st),
                    "rdblquote" => p.push_char('”', &st),
                    _ => {}
                }
            }
            _ => {
                p.flush_ansi(&st);
                p.flush_utf16(&st);
                let rest = &rtf[i..];
                let ch = rest.chars().next().unwrap();
                i += ch.len_utf8();
                if st.dest == Dest::FontTbl && ch == ';' {
                    p.commit_font();
                    continue;
                }
                if st.dest == Dest::FontTbl {
                    p.cur_font_name.push(ch);
                    continue;
                }
                p.push_char(if ch == '\t' { ' ' } else { ch }, &st);
            }
        }
    }

    p.flush_ansi(&st);
    p.flush_utf16(&st);
    p.flush_run(&st);
    if !p.cur_runs.is_empty() || cur_prefix.is_some() || paragraphs.is_empty() {
        paragraphs.push(finalize_paragraph(
            &mut cur_prefix,
            &mut list_kind,
            &mut prev_list_kind,
            &mut list_counter,
            pn_start,
            &mut p.cur_runs,
        ));
    }
    paragraphs
}

fn finalize_paragraph(
    cur_prefix: &mut Option<String>,
    list_kind: &mut Option<ListKind>,
    prev_list_kind: &mut Option<ListKind>,
    list_counter: &mut usize,
    pn_start: usize,
    cur_runs: &mut Vec<Run>,
) -> Paragraph {
    let mut prefix = cur_prefix.take();
    let kind = *list_kind;
    if kind == Some(ListKind::Numbered) {
        if *prev_list_kind == Some(ListKind::Numbered) {
            *list_counter += 1;
        } else {
            *list_counter = pn_start.max(1);
        }
        if prefix.is_none() {
            prefix = Some(format!("{}. ", *list_counter));
        }
    } else {
        *list_counter = 0;
        if kind == Some(ListKind::Bullet) && prefix.is_none() {
            prefix = Some("• ".into());
        }
    }
    *prev_list_kind = kind;
    Paragraph {
        prefix,
        runs: std::mem::take(cur_runs),
    }
}

pub fn note_paragraphs(note: &model::Note, base_body_size: f32) -> (bool, Vec<Paragraph>) {
    let rtf = note.rich_text();
    let paragraphs = parse_rtf(&rtf, base_body_size);
    let has_content = paragraphs
        .iter()
        .any(|p| p.prefix.is_some() || p.runs.iter().any(|r| !r.text.trim().is_empty()));
    (!has_content, paragraphs)
}

fn push_slice(runs: &mut Vec<Run>, style: &Run, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.same_style(style) {
            last.text.push_str(text);
            return;
        }
    }
    let mut r = style.clone();
    r.text = text.to_string();
    runs.push(r);
}

#[derive(Clone)]
struct Slice {
    style: Run,
    text: String,
    width: f32,
}

fn split_soft_lines(runs: &[Run]) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = vec![vec![]];
    for run in runs {
        let mut first = true;
        for part in run.text.split('\n') {
            if !first {
                lines.push(vec![]);
            }
            first = false;
            if !part.is_empty() {
                push_slice(lines.last_mut().unwrap(), run, part);
            }
        }
    }
    lines
}

fn tokenize_line(runs: &[Run], measure: &mut impl FnMut(&Run, &str) -> f32) -> Vec<(bool, Vec<Slice>, f32)> {
    // Returns (is_space, slices, total_width) tokens across runs so word formatting splits don't break words.
    let mut tokens: Vec<(bool, Vec<Slice>, f32)> = Vec::new();
    for run in runs {
        let mut idx = 0usize;
        let s = &run.text;
        while idx < s.len() {
            let ws = s[idx..].chars().next().unwrap().is_whitespace();
            let mut end = idx;
            while end < s.len() {
                let ch = s[end..].chars().next().unwrap();
                if ch.is_whitespace() != ws {
                    break;
                }
                end += ch.len_utf8();
            }
            let chunk = if ws { " " } else { &s[idx..end] };
            let w = measure(run, chunk);
            let slice = Slice {
                style: run.clone(),
                text: chunk.to_string(),
                width: w,
            };
            if let Some((prev_ws, slices, total_w)) = tokens.last_mut() {
                if *prev_ws == ws {
                    if !ws {
                        slices.push(slice);
                        *total_w += w;
                    }
                    idx = end;
                    continue;
                }
            }
            tokens.push((ws, vec![slice], w));
            idx = end;
        }
    }
    tokens
}

pub fn wrap_paragraphs(
    paragraphs: &[Paragraph],
    width: f32,
    base_body: f32,
    measure: &mut impl FnMut(&Run, &str) -> f32,
) -> Vec<RichLine> {
    let avail = (width - 3.).max(base_body);
    let mut out = Vec::new();

    for para in paragraphs {
        let sub_lines = split_soft_lines(&para.runs);
        let pfx_style = para.runs.first().cloned().unwrap_or(Run {
            text: String::new(),
            family: None,
            size_dip: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: None,
        });
        let pfx_run_template = Run {
            underline: false,
            strike: false,
            ..pfx_style
        };
        let indent = if let Some(pfx) = &para.prefix {
            measure(&pfx_run_template, pfx).min(avail * 0.25)
        } else {
            0.
        };

        for (sub_idx, runs) in sub_lines.iter().enumerate() {
            let first_sub_with_prefix = sub_idx == 0 && para.prefix.is_some();
            let mut line_runs: Vec<Run> = Vec::new();
            let mut cur_indent = if first_sub_with_prefix { 0. } else { indent };
            let mut cur_w = if let (true, Some(pfx)) = (first_sub_with_prefix, &para.prefix) {
                push_slice(&mut line_runs, &pfx_run_template, pfx);
                measure(&pfx_run_template, pfx)
            } else {
                0.
            };
            let mut line_has_word = false;
            let tokens = tokenize_line(runs, measure);
            let mut pending_space: Option<Slice> = None;

            for (is_ws, slices, tok_w) in tokens {
                if is_ws {
                    if line_has_word {
                        pending_space = slices.into_iter().next();
                    }
                    continue;
                }
                let space_w = pending_space.as_ref().map_or(0., |s| s.width);
                let line_avail = (avail - cur_indent).max(base_body);
                if cur_w + space_w + tok_w <= line_avail {
                    if let Some(sp) = pending_space.take() {
                        push_slice(&mut line_runs, &sp.style, &sp.text);
                        cur_w += sp.width;
                    }
                    for sl in slices {
                        push_slice(&mut line_runs, &sl.style, &sl.text);
                        cur_w += sl.width;
                    }
                    line_has_word = true;
                    continue;
                }
                pending_space = None;
                if line_has_word {
                    out.push(RichLine {
                        indent: cur_indent,
                        runs: std::mem::take(&mut line_runs),
                    });
                    cur_indent = indent;
                    cur_w = 0.;
                    line_has_word = false;
                }
                let next_avail = (avail - cur_indent).max(base_body);
                if tok_w <= next_avail {
                    for sl in slices {
                        push_slice(&mut line_runs, &sl.style, &sl.text);
                        cur_w += sl.width;
                    }
                    line_has_word = true;
                } else {
                    // Split oversized token by character across its styled slices.
                    for sl in slices {
                        for ch in sl.text.chars() {
                            let mut ch_buf = [0u8; 4];
                            let ch_s = ch.encode_utf8(&mut ch_buf);
                            let cw = measure(&sl.style, ch_s);
                            if line_has_word && cur_w + cw > (avail - cur_indent).max(base_body) {
                                out.push(RichLine {
                                    indent: cur_indent,
                                    runs: std::mem::take(&mut line_runs),
                                });
                                cur_indent = indent;
                                cur_w = 0.;
                            }
                            push_slice(&mut line_runs, &sl.style, ch_s);
                            cur_w += cw;
                            line_has_word = true;
                        }
                    }
                }
            }
            out.push(RichLine {
                indent: cur_indent,
                runs: line_runs,
            });
        }
    }
    out
}

pub unsafe fn wrap_note(
    note: &model::Note,
    text_w: f32,
    dpi: f32,
    body_size: f32,
    typography: &mut typography::Typography,
) -> (bool, Vec<RichLine>) {
    let (empty, paragraphs) = note_paragraphs(note, body_size);
    if empty {
        if note.images.is_empty() {
            return (
                true,
                vec![RichLine {
                    indent: 0.,
                    runs: vec![Run {
                        text: "Write something…".into(),
                        family: None,
                        size_dip: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        strike: false,
                        color: None,
                    }],
                }],
            );
        }
        return (true, vec![]);
    }
    let lines = wrap_paragraphs(&paragraphs, text_w, body_size, &mut |run, s| {
        typography.measure_styled(
            s,
            run.family.as_deref(),
            dpi,
            run.effective_size(body_size),
            run.bold,
            run.italic,
        )
    });
    (false, lines)
}

fn run_d2d_style(run: &Run, body_size: f32, dpi: f32) -> crate::d2d::TextStyle<'_> {
    crate::d2d::TextStyle {
        font_family: run.family.as_deref().unwrap_or("Segoe UI Variable Text"),
        size: run.effective_size(body_size) * dpi,
        weight: if run.bold { 700 } else { 400 },
        italic: run.italic,
        align: crate::d2d::TextAlign::Leading,
        valign: crate::d2d::ParagraphAlign::Center,
        wrap: crate::d2d::WordWrap::NoWrap,
        trim_ellipsis: false,
    }
}

pub fn draw_lines(
    c: &Canvas,
    lines: &[RichLine],
    content_x: f32,
    right_limit: f32,
    top_y: f32,
    leading: f32,
    body_size: f32,
    default_color: u32,
    dpi: f32,
) {
    if lines.is_empty() {
        return;
    }

    let mut d2d_runs: Vec<crate::d2d::RichRun<'_>> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let row_top = top_y + i as f32 * leading;
        let mut cur_x = content_x + line.indent;
        for run in &line.runs {
            let size = run.effective_size(body_size);
            let style_bits = run.style_bits();
            let d2d_style = run_d2d_style(run, body_size, dpi);
            let run_w = crate::d2d::measure_text(
                &run.text,
                &d2d_style,
                100_000.0,
                (leading * dpi).max(1.0),
            )
            .map(|m| m.width_including_trailing_whitespace / dpi)
            .unwrap_or_else(|| c.measure_rich_run(&run.text, run.family.as_deref(), size, style_bits));
            if cur_x < right_limit && !run.text.is_empty() {
                d2d_runs.push(crate::d2d::RichRun {
                    text: &run.text,
                    rect: [
                        (cur_x - content_x) * dpi,
                        (row_top - top_y) * dpi,
                        ((cur_x + run_w + 4.).min(right_limit + 4.) - content_x) * dpi,
                        (row_top + leading - top_y) * dpi,
                    ],
                    style: d2d_style,
                    underline: run.underline,
                    strike: run.strike,
                    color: run.color.unwrap_or(default_color),
                    alpha: 255,
                });
            }
            cur_x += run_w;
        }
    }

    if d2d_runs.is_empty() {
        return;
    }

    let w_dip = (right_limit - content_x + 4.).max(1.);
    let h_dip = (lines.len() as f32 * leading + 4.).max(1.);
    let w_px = (w_dip * dpi).ceil().max(1.) as i32;
    let h_px = (h_dip * dpi).ceil().max(1.) as i32;
    if let Some(buf) = unsafe { crate::compositor::Buffer::plain(w_px, h_px, 1.0) } {
        if unsafe { crate::d2d::draw_rich_runs_dc(buf.dc, w_px, h_px, &d2d_runs) } {
            let mut px = unsafe { buf.snapshot() };
            // Direct2D writes premultiplied BGRA; Image::from_rgba swaps R/B to GDI+ BGRA.
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            if let Some(img) = crate::render::Image::from_rgba(w_px as u32, h_px as u32, px) {
                c.image(
                    &img,
                    [
                        content_x,
                        top_y,
                        content_x + w_px as f32 / dpi,
                        top_y + h_px as f32 / dpi,
                    ],
                );
                return;
            }
        }
    }

    for (i, line) in lines.iter().enumerate() {
        let top = top_y + i as f32 * leading;
        let mut cur_x = content_x + line.indent;
        for run in &line.runs {
            let size = run.effective_size(body_size);
            let style = run.style_bits();
            let color = run.color.unwrap_or(default_color);
            let run_w = c.measure_rich_run(&run.text, run.family.as_deref(), size, style);
            if cur_x < right_limit {
                c.text_rich_run(
                    &run.text,
                    [cur_x, top, (cur_x + run_w + 6.).min(right_limit + 4.), top + leading],
                    run.family.as_deref(),
                    size,
                    style,
                    color,
                    255,
                );
            }
            cur_x += run_w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rtf_preserves_styles_unicode_fonts_and_lists() {
        let rtf = r#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil\fcharset0 Segoe UI Variable Text;}{\f1\fnil\fcharset0 Consolas;}{\f2\fcharset2 Symbol;}}
{\colortbl ;\red255\green80\blue40;}
\pard\f0\fs21 Excepteur moll\i it anim\i0 m.\par
\pard{\pntext\f2\'B7\tab}{\*\pn\pnlvlblt\pnf2\pnindent0{\pntxtb\'B7}}\b Bold\b0  and \ul under\ulnone  \strike cut\strike0  \f1\cf1 code\par
\pard{\pntext\f0 1.\tab}{\*\pn\pnlvlbody\pnf0\pnindent0\pnstart1\pndec}Ti\u7871?ng Vi\u7879?t \u-10179?\u-8704?\par
}"#;
        let paras = parse_rtf(rtf, 14.);
        assert_eq!(paras.len(), 3);
        assert_eq!(paras[0].prefix, None);
        assert_eq!(paras[0].runs.len(), 3);
        assert_eq!(paras[0].runs[0].text, "Excepteur moll");
        assert!(!paras[0].runs[0].italic);
        assert_eq!(paras[0].runs[1].text, "it anim");
        assert!(paras[0].runs[1].italic);
        assert_eq!(paras[0].runs[2].text, "m.");

        assert_eq!(paras[1].prefix.as_deref(), Some("• "));
        assert!(paras[1].runs[0].bold);
        assert!(paras[1].runs.iter().any(|r| r.underline && r.text == "under"));
        assert!(paras[1].runs.iter().any(|r| r.strike && r.text == "cut"));
        let code = paras[1].runs.last().unwrap();
        assert_eq!(code.family.as_deref(), Some("Consolas"));
        assert!(code.color.is_some());

        assert_eq!(paras[2].prefix.as_deref(), Some("1. "));
        let text: String = paras[2].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "Tiếng Việt 😀");
    }

    #[test]
    fn wrap_keeps_styled_midword_boundary_together() {
        let para = Paragraph {
            prefix: None,
            runs: vec![
                Run {
                    text: "deserunt moll".into(),
                    family: None,
                    size_dip: None,
                    bold: false,
                    italic: false,
                    underline: false,
                    strike: false,
                    color: None,
                },
                Run {
                    text: "it anim".into(),
                    family: None,
                    size_dip: None,
                    bold: false,
                    italic: true,
                    underline: false,
                    strike: false,
                    color: None,
                },
            ],
        };
        let lines = wrap_paragraphs(&[para], 15., 1., &mut |_r, s| s.chars().count() as f32);
        assert_eq!(lines.len(), 2);
        let first: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        let second: String = lines[1].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(first, "deserunt");
        assert_eq!(second, "mollit anim");
        assert!(!lines[1].runs[0].italic);
        assert!(lines[1].runs[1].italic);
    }

    #[test]
    fn parse_rtf_decodes_vietnamese_cp1258_and_fcharset163_and_238() {
        let rtf_cpg = r#"{\rtf1\ansi\ansicpg1258{\fonttbl{\f0 Segoe UI Variable Text;}}\f0\fs21 \'ea \'f0 \'d0 a\'ec \'f5\'f2}"#;
        let paras_cpg = parse_rtf(rtf_cpg, 14.);
        let text_cpg: String = paras_cpg[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text_cpg, "ê đ Đ á ợ");
        assert!(!text_cpg.contains('ð'));

        let rtf_cs = r#"{\rtf1\ansi{\fonttbl{\f0 Segoe UI Variable Text;}{\f1\fcharset163 Segoe UI Variable Text;}{\f3\fcharset238 Segoe UI Variable Text;}}\f1 \'f0 \'e3 \'e3\'ec \f3 \'f0}"#;
        let paras_cs = parse_rtf(rtf_cs, 14.);
        let text_cs: String = paras_cs[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text_cs, "đ ă ắ đ");
        assert!(!text_cs.contains('ð'));
        assert!(!text_cs.contains('ã'));
    }
}
