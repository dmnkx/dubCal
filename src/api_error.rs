use serde_json::Value;

pub fn message(status: u16, body: &Value) -> String {
    let error = &body["error"];
    let mut reasons = Vec::new();
    for field in ["errors", "details"] {
        if let Some(entries) = error[field].as_array() {
            for entry in entries {
                if let Some(reason) = entry["reason"].as_str() {
                    reasons.push(reason);
                }
            }
        }
    }
    let has = |values: &[&str]| reasons.iter().any(|reason| values.contains(reason));
    let hint = if has(&["accessNotConfigured", "SERVICE_DISABLED"]) {
        "OAuth 클라이언트를 만든 프로젝트에서 Google Calendar API를 사용 설정해 주세요. 설정 반영 후 새로고침하면 됩니다."
    } else if has(&["insufficientPermissions", "ACCESS_TOKEN_SCOPE_INSUFFICIENT"]) {
        "캘린더 접근 권한이 부족합니다. Google 연결을 다시 눌러 일정 접근 권한을 허용해 주세요."
    } else if has(&[
        "userRateLimitExceeded",
        "rateLimitExceeded",
        "quotaExceeded",
        "RATE_LIMIT_EXCEEDED",
    ]) || status == 429
    {
        "Google Calendar 사용 한도에 도달했습니다. 잠시 후 새로고침해 주세요."
    } else if status == 401 {
        "Google 로그인이 만료되었거나 취소되었습니다. Google 연결을 다시 진행해 주세요."
    } else if status == 403 {
        "Google Calendar 접근이 거부되었습니다. API 사용 설정, 계정의 캘린더 권한과 조직 정책을 확인해 주세요."
    } else {
        "Google Calendar 요청을 처리하지 못했습니다."
    };
    let reason = reasons
        .first()
        .copied()
        .filter(|reason| {
            reason.len() <= 80
                && reason
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .unwrap_or("unknown");
    let detail: String = error["message"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect();
    if detail.is_empty() {
        format!("{hint} (HTTP {status}, {reason})")
    } else {
        format!("{hint}\nGoogle 응답: {detail} (HTTP {status}, {reason})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn disabled_service_and_missing_scope_have_different_instructions() {
        let disabled = message(
            403,
            &json!({ "error": { "errors": [{ "reason": "accessNotConfigured" }], "message": "Calendar API is disabled" } }),
        );
        assert!(disabled.contains("사용 설정"));
        assert!(disabled.contains("Calendar API is disabled"));
        let missing_scope = message(
            403,
            &json!({ "error": { "details": [{ "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT" }] } }),
        );
        assert!(missing_scope.contains("Google 연결을 다시"));
        let limited = message(
            403,
            &json!({ "error": { "errors": [{ "reason": "rateLimitExceeded" }] } }),
        );
        assert!(limited.contains("사용 한도"));
    }
    #[test]
    fn missing_or_non_json_error_body_still_gives_a_readable_message() {
        assert!(message(403, &Value::Null).contains("접근이 거부"));
        assert!(message(401, &Value::Null).contains("로그인"));
    }
}
