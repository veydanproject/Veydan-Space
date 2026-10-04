// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Certificates made for the tests. They guard nothing: the keys are
//! right here. Kept as text in the source so that no rule about key
//! files in a repository hides them from a build.

pub const CERT: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBfTCCASOgAwIBAgIUVRbn8ELjrMUZVR3GbCvCzAmuu2wwCgYIKoZIzj0EAwIw\n\
FDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MTAwMjAxMjgwNVoXDTQ2MDkyNzAx\n\
MjgwNVowFDESMBAGA1UEAwwJbG9jYWxob3N0MFkwEwYHKoZIzj0CAQYIKoZIzj0D\n\
AQcDQgAEWvMegnwpaQDr2J+5r//XAp6IfwWPI15NvM/Zep8+d/U7zqv6B3tKRlVL\n\
KqrMCPRWW83R4DzcbV3Whra+e2j18aNTMFEwHQYDVR0OBBYEFNwHPH8v90MSSlS4\n\
aXPg9rwaea6lMB8GA1UdIwQYMBaAFNwHPH8v90MSSlS4aXPg9rwaea6lMA8GA1Ud\n\
EwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhAPcUJyBmMzjaznqE+XTJZCS8\n\
U732aTtxFmU0XGKW1PzpAiAhKYC2gsI8uUTgagYVj+1tMGRI+P78u2XYStKRqXeG\n\
AA==\n\
-----END CERTIFICATE-----\n\
";

pub const KEY: &str = "\
-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgr4CigaXWBXlVUcsH\n\
aTmo8mcfve+IKk3HzLtpAzQlT2OhRANCAARa8x6CfClpAOvYn7mv/9cCnoh/BY8j\n\
Xk28z9l6nz539TvOq/oHe0pGVUsqqswI9FZbzdHgPNxtXdaGtr57aPXx\n\
-----END PRIVATE KEY-----\n\
";

/// The certificate of another bridge.
pub const OTHER_CERT: &str = "\
-----BEGIN CERTIFICATE-----\n\
MIIBfTCCASOgAwIBAgIUdxZaVcEzoBPuVoJe95muJCaZhnAwCgYIKoZIzj0EAwIw\n\
FDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MTAwMjAxMjgwNVoXDTQ2MDkyNzAx\n\
MjgwNVowFDESMBAGA1UEAwwJbG9jYWxob3N0MFkwEwYHKoZIzj0CAQYIKoZIzj0D\n\
AQcDQgAEn6mcVJ0kXx/8FSo9lIRerVmzhp6QIckBM/OKXQcEGKz5t7WkHvwm35L8\n\
yEobwdwHrJiK/jgtv/CFXcw548ggWKNTMFEwHQYDVR0OBBYEFH8UmnWm/0j8Srl5\n\
KQk7u6VzNPcWMB8GA1UdIwQYMBaAFH8UmnWm/0j8Srl5KQk7u6VzNPcWMA8GA1Ud\n\
EwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIgFfQhXb0K4aVReO8+G4FpnnfM\n\
UBUVBn61HaxFdndqDNkCIQC5vxdeE1SEyZBPyNok1SxjFSzXG+zrifr+vNnwLf9b\n\
GQ==\n\
-----END CERTIFICATE-----\n\
";
