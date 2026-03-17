use std::fs;

/// CRC32 (ISO 3309 / ITU-T V.42) — dependency-free, algorithm-stable implementation.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn main() {
    println!("cargo:rerun-if-changed=shells/zsh/abbrs.zsh");

    let template = fs::read("shells/zsh/abbrs.zsh").expect("failed to read shells/zsh/abbrs.zsh");
    let hash = crc32(&template);

    println!("cargo:rustc-env=ABBRS_INIT_SCRIPT_HASH={hash:08x}");
}
