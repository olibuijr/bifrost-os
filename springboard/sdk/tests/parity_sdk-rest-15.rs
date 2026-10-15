//! Parity sdk-rest-15: headless tests for the cloud account sign-in client and the GPU buffer sharing layout. Nothing here
//! opens a socket, a GPU device or a file. The transport is a canned closure.

use bifrost_app::buffer_share::{choose_import_format, BufferLayout, ShareError, MAX_SIDE, MAX_STRIDE};
use bifrost_app::cloud_account::{normalize_email, parse_sign_in, parse_sign_out, session_token, sign_in, sign_out_request, CloudError, Reply, SignIn};
use bifrost_gpu::dmabuf::{DmaBufFormat, MOD_INVALID, MOD_LINEAR};
use bifrost_gpu::format::PixelFormat;

const BASE: &str = "https://cloud.example.test/api/cloud/";

const ACCOUNT_JSON: &str = r#"{"account":{"id":"a1b2c3","email":"ola@example.is","name":"Ola Bui","avatar":"OB","created_at":1767225600,"is_admin":false}}"#;

// ---- email rules ----------------------------------------------------------------------------------------------------

#[test]
fn an_email_is_trimmed_and_lowercased_before_it_is_sent() {
    assert_eq!(normalize_email("  Ola@Example.IS \n").unwrap(), "ola@example.is");
}

#[test]
fn an_email_without_an_at_sign_or_a_local_part_is_invalid() {
    assert_eq!(normalize_email("ola.example.is"), Err(CloudError::InvalidEmail));
    assert_eq!(normalize_email("@example.is"), Err(CloudError::InvalidEmail));
}

#[test]
fn an_email_domain_needs_a_dot() {
    assert_eq!(normalize_email("ola@localhost"), Err(CloudError::InvalidEmail));
}

#[test]
fn an_email_with_inner_whitespace_is_invalid() {
    assert_eq!(normalize_email("ola bui@example.is"), Err(CloudError::InvalidEmail));
}

#[test]
fn an_email_longer_than_254_bytes_is_invalid() {
    let long = format!("{}@example.is", "a".repeat(250));
    assert_eq!(normalize_email(&long), Err(CloudError::InvalidEmail));
}

// ---- sign-in request ------------------------------------------------------------------------------------------------

#[test]
fn a_sign_in_request_posts_the_normalised_email_and_password_as_json() {
    let creds = SignIn::new("Ola@Example.IS", "hunter2hunter2").unwrap();
    let req = creds.request(BASE).unwrap();
    assert_eq!(req.method, "POST");
    assert_eq!(req.url, "https://cloud.example.test/api/cloud/signin", "one trailing slash on the base is dropped");
    assert_eq!(req.body.as_deref(), Some(br#"{"email":"ola@example.is","password":"hunter2hunter2"}"#.as_slice()));
    let content_type = req.headers.iter().find(|(k, _)| k == "Content-Type").map(|(_, v)| v.as_str());
    assert_eq!(content_type, Some("application/json"));
}

#[test]
fn a_sign_in_request_refuses_a_base_that_is_not_https() {
    let creds = SignIn::new("ola@example.is", "hunter2hunter2").unwrap();
    assert!(matches!(creds.request("http://cloud.example.test/api/cloud"), Err(CloudError::BadBaseURL(_))));
    assert!(matches!(creds.request("https://cloud example.test"), Err(CloudError::BadBaseURL(_))));
}

#[test]
fn an_empty_password_is_refused_before_any_request_is_built() {
    assert_eq!(SignIn::new("ola@example.is", ""), Err(CloudError::EmptyPassword));
}

#[test]
fn the_debug_text_of_a_sign_in_never_shows_the_password() {
    let creds = SignIn::new("ola@example.is", "hunter2hunter2").unwrap();
    let text = format!("{creds:?}");
    assert!(text.contains("ola@example.is"));
    assert!(!text.contains("hunter2"), "the password must stay out of logs: {text}");
}

// ---- sign-in answer -------------------------------------------------------------------------------------------------

#[test]
fn a_successful_sign_in_reads_the_account_and_the_session_cookie() {
    let cookie = "bc_session=tok9f3a; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax";
    let signed = parse_sign_in(200, Some(cookie), ACCOUNT_JSON).unwrap();
    assert_eq!(signed.session_token, "tok9f3a");
    assert_eq!(signed.account.id, "a1b2c3");
    assert_eq!(signed.account.email, "ola@example.is");
    assert_eq!(signed.account.name, "Ola Bui");
    assert_eq!(signed.account.avatar, "OB");
    assert_eq!(signed.account.created_at, 1_767_225_600);
    assert!(!signed.account.is_admin);
}

#[test]
fn a_success_without_a_session_cookie_is_a_malformed_answer() {
    let err = parse_sign_in(200, None, ACCOUNT_JSON).unwrap_err();
    assert!(matches!(err, CloudError::MalformedAnswer(_)), "got {err:?}");
}

#[test]
fn an_empty_session_cookie_from_a_sign_out_is_not_a_session() {
    assert_eq!(session_token("bc_session=; Path=/; Max-Age=0"), None);
    assert_eq!(session_token("other=abc; Path=/"), None);
    assert_eq!(session_token("bc_session=ab cd; Path=/"), None, "a token holds letters and digits only");
}

#[test]
fn a_success_with_a_missing_admin_flag_is_a_malformed_answer() {
    let body = r#"{"account":{"id":"a1","email":"ola@example.is","name":"Ola","avatar":"O","created_at":1}}"#;
    let err = parse_sign_in(200, Some("bc_session=tok"), body).unwrap_err();
    assert!(matches!(err, CloudError::MalformedAnswer(_)), "got {err:?}");
}

#[test]
fn a_wrong_password_is_reported_as_bad_credentials() {
    let body = r#"{"error":"bad_credentials"}"#;
    assert_eq!(parse_sign_in(401, None, body), Err(CloudError::BadCredentials));
}

#[test]
fn a_locked_out_address_is_reported_as_rate_limited() {
    let body = r#"{"error":"rate_limited"}"#;
    assert_eq!(parse_sign_in(429, None, body), Err(CloudError::RateLimited));
}

#[test]
fn a_rejected_email_form_is_reported_as_an_invalid_email() {
    let body = r#"{"error":"bad_email"}"#;
    assert_eq!(parse_sign_in(400, None, body), Err(CloudError::InvalidEmail));
}

#[test]
fn other_server_failures_keep_their_status_and_code() {
    let body = r#"{"error":"internal"}"#;
    assert_eq!(parse_sign_in(500, None, body), Err(CloudError::Rejected { status: 500, code: "internal".into() }));
}

#[test]
fn an_error_body_that_is_not_json_still_reports_its_status() {
    assert_eq!(parse_sign_in(502, None, "<html>bad gateway</html>"), Err(CloudError::Rejected { status: 502, code: String::new() }));
}

#[test]
fn an_answer_larger_than_the_limit_is_refused() {
    let body = "x".repeat(bifrost_app::cloud_account::MAX_ANSWER_BYTES + 1);
    assert!(matches!(parse_sign_in(200, Some("bc_session=t"), &body), Err(CloudError::MalformedAnswer(_))));
}

// ---- transport ------------------------------------------------------------------------------------------------------

#[test]
fn sign_in_sends_one_request_and_reads_the_canned_answer() {
    let creds = SignIn::new("ola@example.is", "hunter2hunter2").unwrap();
    let mut seen = Vec::new();
    let mut send = |req: &bifrost_app::url::URLRequest| -> Result<Reply, String> {
        seen.push(req.url.clone());
        Ok(Reply { status: 200, set_cookie: Some("bc_session=tokZ".into()), body: ACCOUNT_JSON.into() })
    };
    let signed = sign_in(BASE, &creds, &mut send).unwrap();
    assert_eq!(seen, vec!["https://cloud.example.test/api/cloud/signin".to_string()]);
    assert_eq!(signed.session_token, "tokZ");
}

#[test]
fn a_transport_failure_is_reported_with_its_message() {
    let creds = SignIn::new("ola@example.is", "hunter2hunter2").unwrap();
    let mut send = |_: &bifrost_app::url::URLRequest| Err::<Reply, String>("connection refused".into());
    assert_eq!(sign_in(BASE, &creds, &mut send), Err(CloudError::Transport("connection refused".into())));
}

#[test]
fn sign_out_carries_the_session_cookie_and_accepts_no_content() {
    let req = sign_out_request(BASE, "tok9f3a").unwrap();
    assert_eq!(req.method, "POST");
    assert_eq!(req.url, "https://cloud.example.test/api/cloud/signout");
    let cookie = req.headers.iter().find(|(k, _)| k == "Cookie").map(|(_, v)| v.as_str());
    assert_eq!(cookie, Some("bc_session=tok9f3a"));
}

#[test]
fn a_sign_out_answer_of_no_content_ends_the_session() {
    assert_eq!(parse_sign_out(204, ""), Ok(()));
    assert_eq!(parse_sign_out(401, r#"{"error":"unauthorized"}"#), Err(CloudError::Rejected { status: 401, code: "unauthorized".into() }));
}

#[test]
fn a_session_token_that_could_split_a_header_is_refused() {
    assert!(matches!(sign_out_request(BASE, "tok;Path=/"), Err(CloudError::InvalidSession)));
    assert!(matches!(sign_out_request(BASE, ""), Err(CloudError::InvalidSession)));
}

// ---- GPU buffer layout ----------------------------------------------------------------------------------------------

#[test]
fn a_new_layout_has_a_tight_row_stride() {
    let layout = BufferLayout::new(640, 480, PixelFormat::Bgra8Unorm).unwrap();
    assert_eq!(layout.stride(), 2560);
    assert_eq!(layout.byte_len(), 2560 * 480);
}

#[test]
fn a_single_channel_layout_takes_one_byte_per_pixel() {
    let layout = BufferLayout::new(100, 30, PixelFormat::R8Unorm).unwrap();
    assert_eq!(layout.stride(), 100);
    assert_eq!(layout.byte_len(), 3000);
}

#[test]
fn a_float_layout_takes_eight_bytes_per_pixel() {
    let layout = BufferLayout::new(4, 2, PixelFormat::Rgba16Float).unwrap();
    assert_eq!(layout.stride(), 32);
    assert_eq!(layout.byte_len(), 64);
}

#[test]
fn an_empty_side_is_refused() {
    assert_eq!(BufferLayout::new(0, 10, PixelFormat::Bgra8Unorm), Err(ShareError::EmptySize));
    assert_eq!(BufferLayout::new(10, 0, PixelFormat::Bgra8Unorm), Err(ShareError::EmptySize));
}

#[test]
fn a_side_above_the_limit_is_refused_and_the_limit_itself_is_allowed() {
    assert_eq!(BufferLayout::new(MAX_SIDE + 1, 1, PixelFormat::R8Unorm), Err(ShareError::SideTooLarge { side: MAX_SIDE + 1 }));
    assert!(BufferLayout::new(MAX_SIDE, 1, PixelFormat::R8Unorm).is_ok());
}

#[test]
fn a_stride_shorter_than_one_row_is_refused() {
    assert_eq!(
        BufferLayout::with_stride(64, 8, PixelFormat::Bgra8Unorm, 100),
        Err(ShareError::StrideTooSmall { stride: 100, minimum: 256 })
    );
}

#[test]
fn a_stride_above_the_byte_limit_is_refused() {
    let stride = MAX_STRIDE + 1;
    assert_eq!(BufferLayout::with_stride(1, 2, PixelFormat::R8Unorm, stride), Err(ShareError::StrideTooLarge { stride }));
}

#[test]
fn a_stride_that_splits_a_pixel_is_refused() {
    assert_eq!(
        BufferLayout::with_stride(32, 8, PixelFormat::Rgba16Float, 260),
        Err(ShareError::StrideNotPixelAligned { stride: 260, bytes_per_pixel: 8 })
    );
}

#[test]
fn a_padded_stride_sets_the_byte_length() {
    let layout = BufferLayout::with_stride(10, 4, PixelFormat::Bgra8Unorm, 64).unwrap();
    assert_eq!(layout.byte_len(), 256);
}

#[test]
fn a_buffer_must_have_exactly_the_layout_length() {
    let layout = BufferLayout::new(2, 2, PixelFormat::Bgra8Unorm).unwrap();
    assert_eq!(layout.check_bytes(&[0u8; 16]), Ok(()));
    assert_eq!(layout.check_bytes(&[0u8; 15]), Err(ShareError::LengthMismatch { expected: 16, actual: 15 }));
}

#[test]
fn the_fourcc_of_a_bgra_layout_is_the_argb8888_code() {
    let layout = BufferLayout::new(8, 8, PixelFormat::Bgra8Unorm).unwrap();
    assert_eq!(layout.fourcc(), Ok(u32::from_le_bytes(*b"AR24")));
}

// ---- import format choice -------------------------------------------------------------------------------------------

#[test]
fn an_import_takes_the_first_modifier_the_device_lists_for_the_fourcc() {
    let layout = BufferLayout::new(8, 8, PixelFormat::Bgra8Unorm).unwrap();
    let fourcc = layout.fourcc().unwrap();
    let supported = [
        DmaBufFormat { fourcc: u32::from_le_bytes(*b"R8  "), modifier: MOD_LINEAR },
        DmaBufFormat { fourcc, modifier: 0x0100_0000_0000_0001 },
        DmaBufFormat { fourcc, modifier: MOD_LINEAR },
    ];
    let chosen = choose_import_format(&layout, &supported, None).unwrap();
    assert_eq!(chosen, DmaBufFormat { fourcc, modifier: 0x0100_0000_0000_0001 });
}

#[test]
fn an_import_with_a_named_modifier_needs_that_modifier() {
    let layout = BufferLayout::new(8, 8, PixelFormat::Bgra8Unorm).unwrap();
    let fourcc = layout.fourcc().unwrap();
    let supported = [DmaBufFormat { fourcc, modifier: 0x0100_0000_0000_0001 }];
    assert_eq!(choose_import_format(&layout, &supported, Some(MOD_LINEAR)), Err(ShareError::NoImportFormat { fourcc, modifier: MOD_LINEAR }));
    assert!(choose_import_format(&layout, &supported, Some(0x0100_0000_0000_0001)).is_ok());
}

#[test]
fn a_device_without_the_fourcc_refuses_the_import_and_names_it() {
    let layout = BufferLayout::new(8, 8, PixelFormat::Rgba16Float).unwrap();
    let fourcc = layout.fourcc().unwrap();
    let only_linear_r8 = [DmaBufFormat { fourcc: u32::from_le_bytes(*b"R8  "), modifier: MOD_LINEAR }];
    assert_eq!(
        choose_import_format(&layout, &only_linear_r8, None),
        Err(ShareError::NoImportFormat { fourcc, modifier: MOD_INVALID })
    );
}
