//! Names of the files openMapper makes, as OMSI 2 can open them: OMSI 2 reads its files
//! through the system's 8-bit code page, so a letter outside it (Cyrillic on a Western
//! Windows, an accent on a Russian one) names a file it cannot find, and a name with dots
//! before its extension (`lamp~s1.5_2_1.sco`) was not read either. A name made here has
//! only ASCII letters, digits, `_` and `-`; letters with accents and Cyrillic ones are
//! spelt in Latin letters.

/// `name` as a file name OMSI 2 reads (without its extension): empty if nothing of it is left.
pub fn ascii_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if let Some(l) = latin(c) {
            out.push_str(l);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// Whether OMSI 2 opens a file of this name on every system (the folders too, `\` or `/`
/// between them): ASCII letters, digits, `_`, `-`, spaces and one dot, before the extension.
#[cfg(test)]
pub fn readable(rel: &str) -> bool {
    let file = rel.rsplit(['\\', '/']).next().unwrap_or(rel);
    rel.chars().all(|c| c.is_ascii_alphanumeric() || " _-.\\/".contains(c)) && file.matches('.').count() <= 1
}

/// The Latin spelling of a letter with an accent or of a Cyrillic letter.
fn latin(c: char) -> Option<&'static str> {
    Some(match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "A",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'Æ' => "AE",
        'æ' => "ae",
        'Ç' | 'Ć' | 'Č' => "C",
        'ç' | 'ć' | 'č' => "c",
        'Ď' | 'Đ' | 'Ð' => "D",
        'ď' | 'đ' | 'ð' => "d",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => "E",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
        'Ğ' => "G",
        'ğ' => "g",
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'İ' => "I",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => "i",
        'Ł' | 'Ľ' | 'Ĺ' => "L",
        'ł' | 'ľ' | 'ĺ' => "l",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ñ' | 'ń' | 'ň' => "n",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'Œ' => "OE",
        'œ' => "oe",
        'Ř' | 'Ŕ' => "R",
        'ř' | 'ŕ' => "r",
        'Ś' | 'Š' | 'Ş' => "S",
        'ś' | 'š' | 'ş' => "s",
        'ß' => "ss",
        'Ť' | 'Ţ' => "T",
        'ť' | 'ţ' => "t",
        'Þ' => "Th",
        'þ' => "th",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' | 'Ű' => "U",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
        'Ý' | 'Ÿ' => "Y",
        'ý' | 'ÿ' => "y",
        'Ź' | 'Ż' | 'Ž' => "Z",
        'ź' | 'ż' | 'ž' => "z",
        'А' => "A",
        'а' => "a",
        'Б' => "B",
        'б' => "b",
        'В' => "V",
        'в' => "v",
        'Г' | 'Ґ' => "G",
        'г' | 'ґ' => "g",
        'Д' => "D",
        'д' => "d",
        'Е' | 'Ё' | 'Э' => "E",
        'е' | 'ё' | 'э' => "e",
        'Є' => "Ye",
        'є' => "ye",
        'Ж' => "Zh",
        'ж' => "zh",
        'З' => "Z",
        'з' => "z",
        'И' | 'І' => "I",
        'и' | 'і' => "i",
        'Ї' => "Yi",
        'ї' => "yi",
        'Й' | 'Ы' => "Y",
        'й' | 'ы' => "y",
        'К' => "K",
        'к' => "k",
        'Л' => "L",
        'л' => "l",
        'М' => "M",
        'м' => "m",
        'Н' => "N",
        'н' => "n",
        'О' => "O",
        'о' => "o",
        'П' => "P",
        'п' => "p",
        'Р' => "R",
        'р' => "r",
        'С' => "S",
        'с' => "s",
        'Т' => "T",
        'т' => "t",
        'У' => "U",
        'у' => "u",
        'Ф' => "F",
        'ф' => "f",
        'Х' => "Kh",
        'х' => "kh",
        'Ц' => "Ts",
        'ц' => "ts",
        'Ч' => "Ch",
        'ч' => "ch",
        'Ш' => "Sh",
        'ш' => "sh",
        'Щ' => "Shch",
        'щ' => "shch",
        'Ъ' | 'ъ' | 'Ь' | 'ь' => "",
        'Ю' => "Yu",
        'ю' => "yu",
        'Я' => "Ya",
        'я' => "ya",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_spelt_in_ascii() {
        assert_eq!(ascii_stem("Comércio São João"), "Comercio_Sao_Joao");
        assert_eq!(ascii_stem("Остановка №5"), "Ostanovka_5");
        assert_eq!(ascii_stem("lamp~s1.5"), "lamp_s1_5");
        assert_eq!(ascii_stem("  ...  "), "");
        assert_eq!(ascii_stem("Straße-2_b"), "Strasse-2_b");
    }

    #[test]
    fn a_readable_name_has_one_dot_and_ascii() {
        assert!(readable("Sceneryobjects\\Shops\\comercio18.sco"));
        assert!(!readable("Sceneryobjects\\Shops\\comercio18~s1.876_1.436_1.876.sco"));
        assert!(!readable("Splines\\openMapper\\road_bay_right_3.5m.sli"));
        assert!(!readable("Sceneryobjects\\openMapper\\Улица\\a.sco"));
    }
}
