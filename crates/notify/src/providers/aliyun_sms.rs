use super::prelude::*;

pub const PROVIDER: Provider =
    Provider::new("aliyun_sms", "Alibaba Cloud SMS (Aliyun)", FIELDS, build)
        .docs("https://help.aliyun.com/zh/sms/developer-reference/api-dysmsapi-2017-05-25-sendsms")
        .identity(&["phone_numbers", "template_code", "sign_name"])
        .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("access_key_id", "AccessKey ID of a RAM user allowed to send SMS."),
    Field::secret("access_key_secret", "AccessKey secret of that RAM user."),
    Field::list("phone_numbers", "Recipient mobile numbers.").example("13800000000"),
    Field::text(
        "template_code",
        "Approved template code; the template may use `${name}`, `${time}` and `${status}`.",
    )
    .example("SMS_123456789"),
    Field::text("sign_name", "Approved signature name.").example("Vesper"),
    Field::bool(
        "optional_parameters",
        "Also fill `${msg}` with the summary (addresses removed); templates with free text are often rejected.",
    ),
];

const ENDPOINT: &str = "https://dysmsapi.aliyuncs.com/";

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let status = match m.status {
        Status::Down => "DOWN",
        Status::Up => "UP",
        Status::Test => "TEST",
    };
    let mut template = json!({
        "name": truncate(&m.title, 35),
        "time": m.at.format("%Y-%m-%d %H:%M:%S").to_string(),
        "status": status,
    });
    if s.bool("optional_parameters") {
        template["msg"] = json!(scrub(&m.summary()));
    }
    // Aliyun rejects timestamps older than 15 minutes and reused nonces, so
    // both come from the time of building, not the message.
    let now = chrono::Utc::now();
    let phones = s.required_list("phone_numbers")?.join(",");
    let nonce = hex(sha256(format!(
        "{}|{phones}|{}",
        now.timestamp_nanos_opt().unwrap_or_default(),
        m.dedup_key()
    )))[..32]
        .to_string();
    let mut params: Vec<(String, String)> = [
        ("AccessKeyId", s.str("access_key_id")?.to_string()),
        ("Action", "SendSms".into()),
        ("Format", "JSON".into()),
        ("PhoneNumbers", phones),
        ("SignName", s.str("sign_name")?.to_string()),
        ("SignatureMethod", "HMAC-SHA1".into()),
        ("SignatureNonce", nonce),
        ("SignatureVersion", "1.0".into()),
        ("TemplateCode", s.str("template_code")?.to_string()),
        ("TemplateParam", template.to_string()),
        ("Timestamp", now.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        ("Version", "2017-05-25".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let signature = sign("POST", &params, s.str("access_key_secret")?);
    params.push(("Signature".into(), signature));
    Ok(vec![HttpRequest::post(ENDPOINT)
        .form(params)
        .accept(classify)
        .into()])
}

/// RPC signature version 1.0: HMAC-SHA1 over the method, the encoded path
/// `/` and the encoded, sorted, RFC 3986 encoded query, keyed with
/// `secret&`.
fn sign(method: &str, params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let to_sign = format!("{method}&{}&{}", encode("/"), encode(&canonical));
    base64(hmac_sha1(
        format!("{secret}&").as_bytes(),
        to_sign.as_bytes(),
    ))
}

/// Success is `Code: OK`; throttling codes ask to wait.
fn classify(r: &Response) -> Verdict {
    let code = r
        .json()
        .get("Code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if code.contains("LIMIT_CONTROL") || code.starts_with("Throttling") {
        return Verdict::RetryAfter(r.retry_after.unwrap_or(60).max(1));
    }
    body_code(r, "/Code", "OK")
}

/// Replaces URLs, IP addresses and domain names, which Aliyun refuses in
/// template variables, with `[URL]`, `[IP]` and `[Domain]`.
fn scrub(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split(' ')
                .map(scrub_word)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn scrub_word(word: &str) -> String {
    let start = word
        .find(|c: char| c.is_ascii_alphanumeric() || c == '[')
        .unwrap_or(word.len());
    let end = word
        .rfind(|c: char| c.is_ascii_alphanumeric() || c == ']' || c == '/')
        .map_or(start, |i| i + 1)
        .max(start);
    let (prefix, core, suffix) = (&word[..start], &word[start..end], &word[end..]);
    let replacement = if core.contains("://") {
        "[URL]"
    } else if is_ip(core) {
        "[IP]"
    } else if is_domain(core) {
        "[Domain]"
    } else {
        return word.to_string();
    };
    format!("{prefix}{replacement}{suffix}")
}

/// IPv4 with optional port or prefix length, or an IPv6 address.
fn is_ip(text: &str) -> bool {
    let host = text.split(['/']).next().unwrap_or_default();
    let v4 = host.rsplit_once(':').map_or(host, |(h, p)| {
        if p.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    let octets: Vec<&str> = v4.split('.').collect();
    if octets.len() == 4
        && octets
            .iter()
            .all(|o| (1..=3).contains(&o.len()) && o.bytes().all(|b| b.is_ascii_digit()))
    {
        return true;
    }
    let v6 = host.trim_start_matches('[');
    let v6 = v6.split(']').next().unwrap_or_default();
    v6.matches(':').count() >= 2 && v6.parse::<std::net::Ipv6Addr>().is_ok()
}

/// Dot-separated labels ending in an alphabetic top-level label, with an
/// optional port and path.
fn is_domain(text: &str) -> bool {
    let host = text.split('/').next().unwrap_or_default();
    let host = match host.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|t| t.len() >= 2 && t.bytes().all(|b| b.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example of Aliyun's signature documentation (GET request,
    /// AccessKeySecret `testSecret`).
    #[test]
    fn signs_documented_example() {
        let params: Vec<(String, String)> = [
            ("AccessKeyId", "testId"),
            ("Action", "SendSms"),
            ("Format", "XML"),
            ("OutId", "123"),
            ("PhoneNumbers", "15300000001"),
            ("RegionId", "cn-hangzhou"),
            ("SignName", "阿里云短信测试专用"),
            ("SignatureMethod", "HMAC-SHA1"),
            ("SignatureNonce", "45e25e9b-0a6f-4070-8c85-2956eda1b466"),
            ("SignatureVersion", "1.0"),
            ("TemplateCode", "SMS_71390007"),
            ("TemplateParam", r#"{"customer":"test"}"#),
            ("Timestamp", "2017-07-12T02:42:19Z"),
            ("Version", "2017-05-25"),
        ]
        .into_iter()
        .rev()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(
            sign("GET", &params, "testSecret"),
            "zJDF+Lrzhj/ThnlvIToysFRq6t4="
        );
    }

    #[test]
    fn scrubs_addresses() {
        assert_eq!(
            scrub("DOWN Web (web.example.com) at 10.0.0.1:8080, see https://x.io/a"),
            "DOWN Web ([Domain]) at [IP], see [URL]"
        );
        assert_eq!(
            scrub("DOWN Web (jelly/web) since 11:58"),
            "DOWN Web (jelly/web) since 11:58"
        );
        assert_eq!(scrub("v6 [2001:db8::1]:443 ok."), "v6 [IP] ok.");
    }
}
