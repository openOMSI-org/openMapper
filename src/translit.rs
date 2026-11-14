//! File names made from what the user typed, in plain ASCII. OMSI writes its files in
//! Windows-1252, so a Cyrillic name in a `.sli` or a map file comes out as `&#1044;…` and the
//! file it names is never found: Cyrillic letters are spelled in Latin ones, accents dropped.

/// `name` as a file name of ASCII letters, digits, `_` and `-` (empty when nothing is left).
pub fn file_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if let Some(l) = latin(c) {
            out.push_str(&l);
        } else {
            out.push('_');
        }
    }
    out
}

/// A letter spelled in ASCII ones, if it is one that can be.
fn latin(c: char) -> Option<String> {
    let lower = c.to_lowercase().next().unwrap_or(c);
    let l = cyrillic(lower).or_else(|| accented(lower))?;
    if l.is_empty() || lower == c {
        return Some(l.to_string());
    }
    // (an upper case letter: the first of its spelling is)
    let mut s = l[..1].to_ascii_uppercase();
    s.push_str(&l[1..]);
    Some(s)
}

fn cyrillic(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        'і' => "i",
        'ї' => "yi",
        'є' => "ye",
        'ґ' => "g",
        'ў' => "u",
        _ => return None,
    })
}

fn accented(c: char) -> Option<&'static str> {
    Some(match c {
        'ä' => "ae",
        'ö' => "oe",
        'ü' => "ue",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'ø' => "o",
        'à' | 'á' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => "i",
        'ł' | 'ľ' | 'ĺ' => "l",
        'ñ' | 'ń' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ō' | 'ő' => "o",
        'ř' | 'ŕ' => "r",
        'ś' | 'š' | 'ş' | 'ș' => "s",
        'ť' | 'ţ' | 'ț' => "t",
        'ù' | 'ú' | 'û' | 'ū' | 'ů' | 'ű' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_out_in_ascii() {
        assert_eq!(file_name("Main street 4"), "Main_street_4");
        assert_eq!(file_name("Дорога 2+2"), "Doroga_2_2");
        assert_eq!(file_name("Щёлковское шоссе"), "Shchyolkovskoe_shosse");
        assert_eq!(file_name("Їжак"), "Yizhak");
        assert_eq!(file_name("Große Straße"), "Grosse_Strasse");
        assert_eq!(file_name("Łódź"), "Lodz");
        assert_eq!(file_name("道路"), "__");
        assert_eq!(file_name("  "), "");
        for n in ["Дорога", "Ъ", "Ünter-Weg_1"] {
            assert!(file_name(n).chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{n}");
        }
    }
}
