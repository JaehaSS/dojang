//! 모델 출력 텍스트에서 첫 균형 JSON 객체를 추출한다 — `ensemble`·`interview`가 공유하는
//! nonce-이후-JSON-신뢰 파싱 계약의 공통 부분(전신은 `challenge/mod.rs`, R2에서 이관).

/// 텍스트에서 첫 균형 JSON 객체를 추출 (프롬프트 앞뒤 산문 허용). 멀티바이트 안전(char_indices).
pub fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (rel, c) in s[start..].char_indices() {
        let i = start + rel;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&s[start..=i]);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_first_balanced_object_with_surrounding_prose() {
        let s = "여기 결과:\n{\"a\":1,\"b\":\"x}y\"}\n끝.";
        assert_eq!(extract_json_object(s), Some("{\"a\":1,\"b\":\"x}y\"}"));
    }

    #[test]
    fn returns_none_without_braces() {
        assert!(extract_json_object("no json here").is_none());
    }
}
