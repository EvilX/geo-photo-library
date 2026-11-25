//! Настройки пользователя Windows: языки интерфейса и формат дат (kernel32).
//! Вызовы объявлены вручную, чтобы не тянуть крейт `windows-sys` ради трёх функций.
const MUI_LANGUAGE_NAME: u32 = 0x8;
const LOCALE_SSHORTDATE: u32 = 0x1F;
const LOCALE_SSHORTTIME: u32 = 0x79;

#[link(name = "kernel32")]
extern "system" {
    fn GetUserPreferredUILanguages(flags: u32, num: *mut u32, buf: *mut u16, len: *mut u32) -> i32;
    fn GetLocaleInfoEx(locale: *const u16, lctype: u32, data: *mut u16, len: i32) -> i32;
    fn AttachConsole(pid: u32) -> i32;
}

/// Языки интерфейса пользователя по порядку предпочтения: `ru-RU`, `en-US`…
pub fn ui_languages() -> Vec<String> {
    let (mut num, mut len) = (0u32, 0u32);
    // SAFETY: первый вызов с пустым буфером только сообщает нужную длину.
    if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num, std::ptr::null_mut(), &mut len) } == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: буфер той длины, что вернул первый вызов.
    if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num, buf.as_mut_ptr(), &mut len) } == 0 {
        return Vec::new();
    }
    // Список строк через NUL, в конце — двойной NUL.
    buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// Строка из региональных настроек пользователя (`LOCALE_NAME_USER_DEFAULT`).
fn locale_info(lctype: u32) -> Option<String> {
    let mut buf = [0u16; 128];
    // SAFETY: NULL — локаль пользователя; длина буфера передаётся в символах.
    let n = unsafe { GetLocaleInfoEx(std::ptr::null(), lctype, buf.as_mut_ptr(), buf.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

/// Шаблон «дата время» для `glib::DateTime::format` из коротких форматов даты и времени Windows.
pub fn datetime_pattern() -> Option<String> {
    let date = locale_info(LOCALE_SSHORTDATE)?;
    let time = locale_info(LOCALE_SSHORTTIME)?;
    Some(format!("{} {}", convert_pattern(&date), convert_pattern(&time)))
}

/// Формат Windows (`dd.MM.yyyy`, `h:mm tt`) → strftime GLib (`%d.%m.%Y`, `%-I:%M %p`).
/// Секунды и эра выбрасываются, текст в кавычках остаётся как есть.
pub fn convert_pattern(win: &str) -> String {
    let chars: Vec<char> = win.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '%' {
                    out.push('%');
                }
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let spec = match (c, n) {
            ('d', 1) => "%-d",
            ('d', 2) => "%d",
            ('d', 3) => "%a",
            ('d', _) => "%A",
            ('M', 1) => "%-m",
            ('M', 2) => "%m",
            ('M', 3) => "%b",
            ('M', _) => "%B",
            ('y', 1 | 2) => "%y",
            ('y', _) => "%Y",
            ('h', 1) => "%-I",
            ('h', _) => "%I",
            ('H', 1) => "%-H",
            ('H', _) => "%H",
            ('m', 1) => "%-M",
            ('m', _) => "%M",
            ('t', _) => "%p",
            ('s' | 'g', _) => "",
            ('%', _) => {
                out.push_str(&"%%".repeat(n));
                i += n;
                continue;
            }
            _ => {
                out.extend(std::iter::repeat_n(c, n));
                i += n;
                continue;
            }
        };
        out.push_str(spec);
        i += n;
    }
    // После выброшенных секунд остаются разделители: «HH:mm:ss» → «%H:%M:».
    clean_separators(&out)
}

fn clean_separators(s: &str) -> String {
    let mut out = s.trim().to_string();
    for sep in [":", ".", " :", " ."] {
        while let Some(stripped) = out.strip_suffix(sep) {
            out = stripped.trim_end().to_string();
        }
    }
    out.replace(": ", " ").replace("  ", " ")
}

/// Подключить вывод к консоли, из которой запущена программа: у приложения с подсистемой
/// `windows` своей консоли нет, и без этого `--scan` ничего бы не печатал.
pub fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: без аргументов-указателей; ошибка (нет родительской консоли) не страшна.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(convert_pattern("dd.MM.yyyy"), "%d.%m.%Y");
        assert_eq!(convert_pattern("M/d/yyyy"), "%-m/%-d/%Y");
        assert_eq!(convert_pattern("h:mm tt"), "%-I:%M %p");
        assert_eq!(convert_pattern("HH:mm"), "%H:%M");
        assert_eq!(convert_pattern("HH:mm:ss"), "%H:%M");
        assert_eq!(convert_pattern("d/M/yyyy"), "%-d/%-m/%Y");
        assert_eq!(convert_pattern("yyyy-MM-dd"), "%Y-%m-%d");
        assert_eq!(convert_pattern("dd MMM yy"), "%d %b %y");
        assert_eq!(convert_pattern("yyyy'年'M'月'd'日'"), "%Y年%-m月%-d日");
    }
}
