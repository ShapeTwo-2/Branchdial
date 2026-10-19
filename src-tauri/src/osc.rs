//! Extracts Branchdial's shell-integration markers from the PTY byte stream.
//!
//! The prompt hook prints `ESC ] 6973 ; P ; <nonce> ; <exit> ; <cwd> BEL`
//! before every prompt. The parser removes these sequences (so they never
//! reach xterm) and reports the ones carrying the right nonce. Sequences can
//! be split across reads, so a possible partial prefix is held back.

const PREFIX: &[u8] = b"\x1b]6973;";
/// Longer than this and it is not ours; flush it as ordinary output.
const MAX_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub exit_code: i32,
    pub cwd: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Segment {
    Output(Vec<u8>),
    Prompt(Prompt),
}

pub struct MarkerParser {
    nonce: String,
    held: Vec<u8>,
}

impl MarkerParser {
    pub fn new(nonce: impl Into<String>) -> Self {
        Self { nonce: nonce.into(), held: Vec::new() }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Segment> {
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(chunk);

        let mut segments = Vec::new();
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                out.push(data[i]);
                i += 1;
                continue;
            }
            let rest = &data[i..];
            let n = rest.len().min(PREFIX.len());
            if rest[..n] != PREFIX[..n] {
                out.push(data[i]);
                i += 1;
                continue;
            }
            if rest.len() < PREFIX.len() {
                // Could be the start of a marker; wait for more bytes.
                self.held = rest.to_vec();
                break;
            }
            // Terminated by BEL or ST (ESC \).
            let body_start = i + PREFIX.len();
            let end = data[body_start..].iter().enumerate().find_map(|(k, &b)| match b {
                0x07 => Some((body_start + k, 1)),
                0x1b if data.get(body_start + k + 1) == Some(&b'\\') => Some((body_start + k, 2)),
                _ => None,
            });
            match end {
                Some((body_end, term_len)) => {
                    if let Some(p) = self.parse_body(&data[body_start..body_end]) {
                        if !out.is_empty() {
                            segments.push(Segment::Output(std::mem::take(&mut out)));
                        }
                        segments.push(Segment::Prompt(p));
                    }
                    // Ours or a forgery: either way it never reaches the screen.
                    i = body_end + term_len;
                }
                None if rest.len() > MAX_LEN => {
                    out.push(data[i]);
                    i += 1;
                }
                None => {
                    self.held = rest.to_vec();
                    break;
                }
            }
        }
        if !out.is_empty() {
            segments.push(Segment::Output(out));
        }
        segments
    }

    fn parse_body(&self, body: &[u8]) -> Option<Prompt> {
        let body = std::str::from_utf8(body).ok()?;
        let mut parts = body.splitn(4, ';');
        if parts.next()? != "P" || parts.next()? != self.nonce {
            return None;
        }
        let exit_code = parts.next()?.parse().ok()?;
        let cwd = parts.next().unwrap_or("").to_string();
        Some(Prompt { exit_code, cwd })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(nonce: &str, code: i32, cwd: &str) -> Vec<u8> {
        format!("\x1b]6973;P;{nonce};{code};{cwd}\x07").into_bytes()
    }

    fn collect(p: &mut MarkerParser, chunks: &[&[u8]]) -> (Vec<u8>, Vec<Prompt>) {
        let mut out = Vec::new();
        let mut prompts = Vec::new();
        for c in chunks {
            for s in p.feed(c) {
                match s {
                    Segment::Output(b) => out.extend(b),
                    Segment::Prompt(pr) => prompts.push(pr),
                }
            }
        }
        (out, prompts)
    }

    #[test]
    fn extracts_marker_and_keeps_output() {
        let mut p = MarkerParser::new("n1");
        let mut input = b"hello\r\n".to_vec();
        input.extend(marker("n1", 1, "C:/repo;with;semis"));
        input.extend(b"$ ");
        let (out, prompts) = collect(&mut p, &[&input]);
        assert_eq!(out, b"hello\r\n$ ");
        assert_eq!(prompts, vec![Prompt { exit_code: 1, cwd: "C:/repo;with;semis".into() }]);
    }

    #[test]
    fn handles_marker_split_at_every_byte() {
        let mut input = b"ab".to_vec();
        input.extend(marker("xyz", 0, "/home/u"));
        input.extend(b"cd");
        let chunks: Vec<&[u8]> = input.chunks(1).collect();
        let mut p = MarkerParser::new("xyz");
        let (out, prompts) = collect(&mut p, &chunks);
        assert_eq!(out, b"abcd");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].cwd, "/home/u");
    }

    #[test]
    fn wrong_nonce_is_stripped_but_not_reported() {
        let mut p = MarkerParser::new("good");
        let (out, prompts) = collect(&mut p, &[&marker("evil", 0, "/")]);
        assert!(out.is_empty());
        assert!(prompts.is_empty());
    }

    #[test]
    fn other_escape_sequences_pass_through() {
        let mut p = MarkerParser::new("n");
        let input = b"\x1b[32mgreen\x1b[0m \x1b]0;title\x07 \x1b]697;x\x07";
        let (out, prompts) = collect(&mut p, &[input]);
        assert_eq!(out, input.to_vec());
        assert!(prompts.is_empty());
    }

    #[test]
    fn accepts_st_terminator() {
        let mut p = MarkerParser::new("n");
        let (_, prompts) = collect(&mut p, &[b"\x1b]6973;P;n;130;/x\x1b\\"]);
        assert_eq!(prompts, vec![Prompt { exit_code: 130, cwd: "/x".into() }]);
    }

    #[test]
    fn trailing_lone_escape_is_held_then_released() {
        let mut p = MarkerParser::new("n");
        let (out, _) = collect(&mut p, &[b"a\x1b", b"[0m"]);
        assert_eq!(out, b"a\x1b[0m");
    }
}
