use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("google_sheets", "Google Sheets", FIELDS, build)
    .docs("https://developers.google.com/apps-script/guides/web")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Web app URL of a Google Apps Script deployed with access for anyone, which appends each posted row to the sheet.",
)
.example("https://script.google.com/macros/s/AKfycbx123/exec")];

/// Posts the row fields Uptime Kuma's sample Apps Script reads, so the same
/// script works: `timestamp`, `status`, `monitorName`, `monitorUrl`,
/// `message`, `responseTime` and `statusCode`.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "timestamp": m.at.to_rfc3339(),
        "status": m.status_word(),
        "monitorName": m.title,
        "monitorUrl": m.link,
        "message": m.summary(),
        "responseTime": "N/A",
        "statusCode": "N/A",
    });
    let request = HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .accept(classify);
    Ok(vec![request.into()])
}

/// A web app runs `doPost` and then redirects to its output on
/// script.googleusercontent.com; a redirect anywhere else (such as a Google
/// sign-in page) means the deployment is not open to anyone.
fn classify(r: &Response) -> Verdict {
    match r.status {
        302 | 303 => {
            if String::from_utf8_lossy(&r.body).contains("script.googleusercontent.com") {
                Verdict::Accepted
            } else {
                Verdict::Rejected(
                    "redirected away from the script; deploy it with access for anyone".into(),
                )
            }
        }
        _ => by_status(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, body: &str) -> Response {
        Response {
            status,
            retry_after: None,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn accepts_redirect_to_script_output_only() {
        let moved = r#"<HTML><HEAD><TITLE>Moved Temporarily</TITLE></HEAD><BODY>The document has moved <A HREF="https://script.googleusercontent.com/macros/echo?user_content_key=abc">here</A>.</BODY></HTML>"#;
        assert_eq!(classify(&resp(302, moved)), Verdict::Accepted);
        let login = r#"<A HREF="https://accounts.google.com/ServiceLogin?continue=x">here</A>"#;
        assert!(matches!(classify(&resp(302, login)), Verdict::Rejected(_)));
        assert_eq!(
            classify(&resp(200, r#"{"result":"success"}"#)),
            Verdict::Accepted
        );
    }
}
