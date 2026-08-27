//! A minimal Rust v0 symbol demangler for the backtrace.
//!
//! The v0 scheme (`_R...`) encodes paths as a sequence of length-prefixed
//! name strings interleaved with structural markers.  Names appear in
//! outermost-first order (crate → modules → function).  Generic parameters
//! follow the `E` (end-of-generics) marker.
//!
//! The key challenge: after `C` (crate root), the disambiguator is a
//! base62 string that can contain digits.  The boundary between
//! disambiguator and the first name's length is found by trying
//! disambiguator lengths from longest (13) to shortest (2) and picking the
//! first where the remainder starts with a valid "length + name" sequence.
//! The compiler guarantees the longest valid split is the correct one.
//!
//! No allocation: the demangled name is written into a caller-provided
//! buffer (80 bytes — enough for our server symbols).

/// Maximum disambiguator length (12-char type hash + 1-char version).
const MAX_DISAMBIG: usize = 13;
/// Minimum disambiguator length.
const MIN_DISAMBIG: usize = 2;

/// Demangle a Rust v0 mangled name into `buf` (NUL-terminated).  Returns
/// `true` if the name was recognised as a v0 symbol and demangled;
/// `false` if it is not a v0 symbol (the caller should use the raw name).
pub(crate) fn demangle(name: &str, buf: &mut [u8]) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() < 3 || &bytes[..2] != b"_R" {
        return false;
    }

    let mut pos = 2;
    // Collect name slices. The v0 encoding is outermost-first, so the order
    // of appearance is the display order (no reversal needed).
    let mut names: [*const u8; 16] = [core::ptr::null(); 16];
    let mut lens: [usize; 16] = [0; 16];
    let mut nnames = 0;
    // Track where generics start: the first `N`/`S` marker after the crate.
    let mut gen_start: Option<usize> = None;
    // Set once we've passed the crate root.
    let mut past_crate = false;

    loop {
        if pos >= bytes.len() || nnames == 16 {
            break;
        }
        match bytes[pos] {
            // Crate root: C (v|C<disamb>) <len> <name>
            b'C' => {
                pos += 1;
                // Read the disambiguator (or `v` for none).
                if !read_crate_disamb(bytes, &mut pos) {
                    break;
                }
                if !read_name(bytes, &mut pos, &mut names[nnames], &mut lens[nnames]) {
                    break;
                }
                nnames += 1;
                past_crate = true;
            }
            // Path segment: (N|S|I|M) (v|C<disamb>) <len> <name>
            b'N' | b'S' | b'I' | b'M' => {
                // A nested name after the crate marks the start of generics.
                if past_crate && gen_start.is_none() && nnames > 0 {
                    gen_start = Some(nnames);
                }
                pos += 1;
                // May be followed by a namespace marker (t, v, m, etc.).
                if pos < bytes.len()
                    && matches!(
                        bytes[pos],
                        b't' | b'v' | b'm' | b'b' | b'l' | b'o' | b'p' | b'y' | b'q'
                    )
                {
                    pos += 1;
                }
                // If the next char is another structural marker, let the main
                // loop handle it (e.g. `Nt` before a crate root).
                if pos < bytes.len()
                    && matches!(bytes[pos], b'N' | b'S' | b'I' | b'M' | b'C' | b'E' | b'B')
                {
                    continue;
                }
                // May have a disambiguator: C<disamb> or v.
                if pos < bytes.len() && bytes[pos] == b'C' {
                    pos += 1;
                    if !read_crate_disamb(bytes, &mut pos) {
                        break;
                    }
                } else if pos < bytes.len() && bytes[pos] == b'v' {
                    pos += 1;
                }
                if !read_name(bytes, &mut pos, &mut names[nnames], &mut lens[nnames]) {
                    break;
                }
                nnames += 1;
            }
            // Opaque type: B + optional disambiguator.
            b'B' => {
                pos += 1;
                if pos < bytes.len() && bytes[pos] == b'C' {
                    let _ = read_crate_disamb(bytes, &mut pos);
                }
            }
            // End of generics / bare namespace markers: skip.
            b'E' | b't' | b'v' | b'm' | b'b' | b'l' | b'o' | b'p' | b'y' | b'q' => {
                pos += 1;
            }
            // A bare name (module or function in the path): <len> <name>.
            c if c.is_ascii_digit() => {
                if !read_name(bytes, &mut pos, &mut names[nnames], &mut lens[nnames]) {
                    break;
                }
                nnames += 1;
            }
            _ => break,
        }
    }

    if nnames == 0 {
        return false;
    }

    // Format the output: names before `E` joined with `::`, then generic
    // params (names after `E`) in `<>` if present.
    let mut out_pos = 0;
    let main_count = gen_start.unwrap_or(nnames);

    for i in 0..main_count {
        if out_pos > 0 {
            if out_pos + 2 > buf.len() {
                break;
            }
            buf[out_pos] = b':';
            buf[out_pos + 1] = b':';
            out_pos += 2;
        }
        out_pos += copy_name(names[i], lens[i], &mut buf[out_pos..]);
    }

    // Generic params in ::<>.
    if let Some(gs) = gen_start
        && gs < nnames
    {
        if out_pos + 3 < buf.len() {
            buf[out_pos] = b':';
            buf[out_pos + 1] = b':';
            buf[out_pos + 2] = b'<';
            out_pos += 3;
        }
        for i in gs..nnames {
            if out_pos > 0 && buf[out_pos - 1] != b'<' {
                if out_pos + 2 > buf.len() {
                    break;
                }
                buf[out_pos] = b':';
                buf[out_pos + 1] = b':';
                out_pos += 2;
            }
            out_pos += copy_name(names[i], lens[i], &mut buf[out_pos..]);
        }
        if out_pos < buf.len() {
            buf[out_pos] = b'>';
            out_pos += 1;
        }
    }

    if out_pos < buf.len() {
        buf[out_pos] = 0;
    }
    true
}

/// Read a crate's disambiguator: `v` (none) or a base62 string (2-13 chars).
/// The boundary is found by trying lengths from longest to shortest: the
/// first where the remainder starts with a valid "length + name" wins.
fn read_crate_disamb(bytes: &[u8], pos: &mut usize) -> bool {
    if *pos >= bytes.len() {
        return false;
    }
    if bytes[*pos] == b'v' {
        *pos += 1;
        return true;
    }
    // Try disambiguator lengths from longest to shortest.
    let start = *pos;
    for dlen in (MIN_DISAMBIG..=MAX_DISAMBIG).rev() {
        if start + dlen >= bytes.len() {
            continue;
        }
        // The disambiguator must be all base62 chars.
        if !bytes[start..start + dlen].iter().all(|&b| is_disamb_byte(b)) {
            continue;
        }
        // The remainder must start with a valid "length + name".
        let rem = start + dlen;
        if !bytes[rem].is_ascii_digit() {
            continue;
        }
        let mut p = rem;
        if !peek_name(bytes, &mut p) {
            continue;
        }
        // Valid split: the disambiguator is bytes[start..rem].
        *pos = rem;
        return true;
    }
    false
}

/// Read a `<decimal_len><name_bytes>` sequence at `*pos`.  Advances `*pos`
/// past the name.  Sets `out_ptr` (to the name bytes) and `out_len`.
fn read_name(bytes: &[u8], pos: &mut usize, out_ptr: &mut *const u8, out_len: &mut usize) -> bool {
    let digit_start = *pos;
    let mut p = *pos;
    if !peek_name(bytes, &mut p) {
        return false;
    }
    // After peek_name, p is at the start of the name (past the digits).
    // The digits are at digit_start..p.
    let len: usize =
        bytes[digit_start..p].iter().fold(0usize, |acc, &b| acc * 10 + (b - b'0') as usize);
    // SAFETY: p is within `bytes` (checked in peek_name).
    *out_ptr = unsafe { bytes.as_ptr().add(p) };
    *out_len = len;
    *pos = p + len;
    true
}

/// Try to read a `<decimal_len><name_bytes>` at `*pos`.  On success,
/// advances `*pos` past the name and sets `*out_len`.  Returns `false` if
/// no valid name is here.
fn peek_name(bytes: &[u8], pos: &mut usize) -> bool {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return false;
    }
    let len: usize =
        bytes[start..*pos].iter().fold(0usize, |acc, &b| acc * 10 + (b - b'0') as usize);
    if len == 0 || len > 255 || *pos + len > bytes.len() {
        *pos = start;
        return false;
    }
    if !bytes[*pos..*pos + len].iter().all(|&b| is_name_byte(b)) {
        *pos = start;
        return false;
    }
    true
}

/// Copy a name slice into `dst`, returning the number of bytes written.
fn copy_name(src: *const u8, len: usize, dst: &mut [u8]) -> usize {
    let n = len.min(dst.len());
    // SAFETY: src points into the input string, valid for the call's duration.
    let s = unsafe { core::slice::from_raw_parts(src, n) };
    dst[..n].copy_from_slice(s);
    n
}

/// A valid disambiguator byte: base62 + underscore.
fn is_disamb_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A valid name byte: alphanumeric or underscore.
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_demangle(s: &str) -> String {
        let mut buf = [0u8; 80];
        if demangle(s, &mut buf) {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..len]).into_owned()
        } else {
            format!("<not demangled: {s}>")
        }
    }

    #[test]
    fn demangles_crate_function() {
        assert_eq!(try_demangle("_RNvCs3CFL12ppSuB_9faulttest4main"), "faulttest::main");
    }

    #[test]
    fn demangles_crate_module_function() {
        assert_eq!(
            try_demangle("_RNvNtCsap4Ua6k489x_7r9x_std7process4exit"),
            "r9x_std::process::exit"
        );
    }
}
