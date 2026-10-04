// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! AWS Signature Version 4 for S3-compatible storage (MinIO, Ceph, AWS).
//! Header-based signing of single requests; no SDK, no chunked signing.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub struct Credentials<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
}

/// One request to sign. `headers` must contain `host`; `x-amz-date` and
/// `x-amz-content-sha256` are added by `sign`.
pub struct Request<'a> {
    pub method: &'a str,
    /// Absolute path, not yet encoded (`/bucket/key`).
    pub path: &'a str,
    /// Query pairs, not yet encoded.
    pub query: &'a [(&'a str, &'a str)],
    pub headers: Vec<(String, String)>,
    /// Hex SHA-256 of the body, or `UNSIGNED_PAYLOAD`.
    pub payload_sha256: &'a str,
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// RFC 3986 encoding as S3 wants it; `/` is kept in paths only.
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `(YYYYMMDD'T'HHMMSS'Z', YYYYMMDD)` for unix seconds.
pub fn amz_date(unix: i64) -> (String, String) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let date = format!("{y:04}{m:02}{d:02}");
    (format!("{date}T{:02}{:02}{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60), date)
}

/// Returns the headers to send: the given ones plus `x-amz-date`,
/// `x-amz-content-sha256` and `authorization`.
pub fn sign(creds: &Credentials<'_>, req: Request<'_>, unix_now: i64) -> Vec<(String, String)> {
    let (stamp, date) = amz_date(unix_now);
    let mut headers: Vec<(String, String)> =
        req.headers.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string())).collect();
    headers.push(("x-amz-date".into(), stamp.clone()));
    headers.push(("x-amz-content-sha256".into(), req.payload_sha256.to_string()));
    headers.sort();

    let mut query: Vec<(String, String)> =
        req.query.iter().map(|(k, v)| (uri_encode(k, false), uri_encode(v, false))).collect();
    query.sort();
    let canonical_query = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method,
        uri_encode(req.path, true),
        canonical_query,
        canonical_headers,
        signed_headers,
        req.payload_sha256
    );
    let scope = format!("{date}/{}/s3/aws4_request", creds.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let k_date = hmac(format!("AWS4{}", creds.secret_key).as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, creds.region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    headers.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            creds.access_key
        ),
    ));
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    /// 2013-05-24T00:00:00Z, the date of the examples in the S3 docs.
    const T: i64 = 1_369_353_600;

    fn auth(h: &[(String, String)]) -> String {
        h.iter().find(|(k, _)| k == "authorization").unwrap().1.clone()
    }

    #[test]
    fn date_formatting() {
        assert_eq!(amz_date(T), ("20130524T000000Z".to_string(), "20130524".to_string()));
        assert_eq!(amz_date(0).0, "19700101T000000Z");
        assert_eq!(amz_date(1_790_640_000).1.len(), 8);
        assert_eq!(amz_date(951_782_400 + 86_399).0, "20000229T235959Z", "leap day");
    }

    #[test]
    fn s3_documentation_get_object_vector() {
        let creds = Credentials { access_key: ACCESS, secret_key: SECRET, region: "us-east-1" };
        let headers = sign(
            &creds,
            Request {
                method: "GET",
                path: "/test.txt",
                query: &[],
                headers: vec![
                    ("Host".into(), "examplebucket.s3.amazonaws.com".into()),
                    ("Range".into(), "bytes=0-9".into()),
                ],
                payload_sha256: EMPTY_SHA256,
            },
            T,
        );
        assert!(auth(&headers).ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"), "{}", auth(&headers));
        assert!(auth(&headers).contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn s3_documentation_list_objects_vector() {
        let creds = Credentials { access_key: ACCESS, secret_key: SECRET, region: "us-east-1" };
        let headers = sign(
            &creds,
            Request {
                method: "GET",
                path: "/",
                query: &[("max-keys", "2"), ("prefix", "J")],
                headers: vec![("host".into(), "examplebucket.s3.amazonaws.com".into())],
                payload_sha256: EMPTY_SHA256,
            },
            T,
        );
        assert!(auth(&headers).ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"), "{}", auth(&headers));
    }

    #[test]
    fn encoding() {
        assert_eq!(uri_encode("/a b/c+d", true), "/a%20b/c%2Bd");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("Ab-_.~9", false), "Ab-_.~9");
    }
}
