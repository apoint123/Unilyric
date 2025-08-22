//! 本模块用于解密 QQ 音乐的加密 QRC 歌词格式。
//!
//! **警告**：
//! 该 DES 实现并非标准实现！
//! 它是结构类似DES的、但完全私有的分组密码算法。
//! 本实现仅用于 QRC 歌词解密，不应用于实际安全目的。
//!
//! ## 致谢
//!
//! - Brad Conte 的原始 DES 实现。
//! - `LyricDecoder` 项目针对 QQ 音乐的改编。
//!
//! - Copyright (c) `SuJiKiNen` (`LyricDecoder` Project)
//! - Licensed under the MIT License.
//!
//! <https://github.com/SuJiKiNen/LyricDecoder>

use crate::error::Result;

////////////////////////////////////////////////////////////////////////////////////////////////////

/// 对加密文本执行解密操作。
pub fn decrypt_qrc(encrypted_text: &str) -> Result<String> {
    let decrypted_string = qrc_logic::decrypt_lyrics(encrypted_text)?;
    Ok(decrypted_string)
}

/// 对明文歌词执行加密操作。
pub fn encrypt_qrc(plaintext: &str) -> Result<String> {
    let encrypted_hex_string = qrc_logic::encrypt_lyrics(plaintext)?;
    Ok(encrypted_hex_string)
}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// 内部模块，封装了所有解密逻辑。
mod qrc_logic {
    use super::Result;
    use crate::error::LyricsHelperError;
    use flate2::Compression;
    use flate2::read::ZlibDecoder;
    use flate2::write::ZlibEncoder;
    use hex::{decode, encode};
    use std::io::{Read, Write};
    use std::sync::LazyLock;

    static CODEC: LazyLock<QqMusicCodec> = LazyLock::new(QqMusicCodec::new);

    const ROUNDS: usize = 16;
    const SUB_KEY_SIZE: usize = 6;
    type TripleDesKeySchedules = [[[u8; SUB_KEY_SIZE]; ROUNDS]; 3];

    const DES_BLOCK_SIZE: usize = 8;

    /// 非标准 3DES 编解码器
    struct QqMusicCodec {
        encrypt_schedule: TripleDesKeySchedules,
        decrypt_schedule: TripleDesKeySchedules,
    }

    impl QqMusicCodec {
        fn new() -> Self {
            const ENCRYPT_OPS: [(&[u8; 8], custom_des::Mode); 3] = [
                (custom_des::KEY_1, custom_des::Mode::Encrypt),
                (custom_des::KEY_2, custom_des::Mode::Decrypt),
                (custom_des::KEY_3, custom_des::Mode::Encrypt),
            ];

            const DECRYPT_OPS: [(&[u8; 8], custom_des::Mode); 3] = [
                (custom_des::KEY_3, custom_des::Mode::Decrypt),
                (custom_des::KEY_2, custom_des::Mode::Encrypt),
                (custom_des::KEY_1, custom_des::Mode::Decrypt),
            ];

            fn generate_schedules(ops: [(&[u8; 8], custom_des::Mode); 3]) -> TripleDesKeySchedules {
                let mut schedules: TripleDesKeySchedules = [[[0; SUB_KEY_SIZE]; ROUNDS]; 3];
                for (i, (key, mode)) in ops.iter().enumerate() {
                    custom_des::key_schedule(*key, &mut schedules[i], *mode);
                }
                schedules
            }

            Self {
                encrypt_schedule: generate_schedules(ENCRYPT_OPS),
                decrypt_schedule: generate_schedules(DECRYPT_OPS),
            }
        }

        /// 加密一个8字节的数据块。
        fn encrypt_block(&self, input: &[u8], output: &mut [u8]) {
            let mut temp1 = [0u8; 8];
            let mut temp2 = [0u8; 8];
            custom_des::des_crypt(input, &mut temp1, &self.encrypt_schedule[0]);
            custom_des::des_crypt(&temp1, &mut temp2, &self.encrypt_schedule[1]);
            custom_des::des_crypt(&temp2, output, &self.encrypt_schedule[2]);
        }

        /// 解密一个8字节的数据块。
        fn decrypt_block(&self, input: &[u8], output: &mut [u8]) {
            let mut temp1 = [0u8; 8];
            let mut temp2 = [0u8; 8];
            custom_des::des_crypt(input, &mut temp1, &self.decrypt_schedule[0]);
            custom_des::des_crypt(&temp1, &mut temp2, &self.decrypt_schedule[1]);
            custom_des::des_crypt(&temp2, output, &self.decrypt_schedule[2]);
        }
    }

    /// 解密 QQ 音乐歌词的主函数
    pub(super) fn decrypt_lyrics(encrypted_hex_str: &str) -> Result<String> {
        let encrypted_bytes = decode(encrypted_hex_str)
            .map_err(|e| LyricsHelperError::Decryption(format!("无效的十六进制字符串: {e}")))?;

        if encrypted_bytes.len() % DES_BLOCK_SIZE != 0 {
            return Err(LyricsHelperError::Decryption(format!(
                "加密数据长度不是{DES_BLOCK_SIZE}的倍数",
            )));
        }

        let mut decrypted_data = vec![0; encrypted_bytes.len()];

        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            decrypted_data
                .par_chunks_mut(DES_BLOCK_SIZE)
                .zip(encrypted_bytes.par_chunks(DES_BLOCK_SIZE))
                .for_each(|(out_slice, chunk)| {
                    CODEC.decrypt_block(chunk, out_slice);
                });
        }

        #[cfg(target_arch = "wasm32")]
        {
            decrypted_data
                .chunks_mut(DES_BLOCK_SIZE)
                .zip(encrypted_bytes.chunks(DES_BLOCK_SIZE))
                .for_each(|(out_slice, chunk)| {
                    CODEC.decrypt_block(chunk, out_slice);
                });
        }

        let decompressed_bytes = decompress(&decrypted_data)?;

        String::from_utf8(decompressed_bytes)
            .map_err(|e| LyricsHelperError::Decryption(format!("UTF-8编码转换失败: {e}")))
    }

    /// 加密 QQ 音乐歌词的主函数
    pub(super) fn encrypt_lyrics(plaintext: &str) -> Result<String> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(plaintext.as_bytes())
            .map_err(|e| LyricsHelperError::Encryption(format!("Zlib压缩写入失败: {e}")))?;
        let compressed_data = encoder
            .finish()
            .map_err(|e| LyricsHelperError::Encryption(format!("Zlib压缩完成失败: {e}")))?;

        let padded_data = zero_pad(&compressed_data, DES_BLOCK_SIZE);

        let mut encrypted_data = vec![0; padded_data.len()];

        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            encrypted_data
                .par_chunks_mut(DES_BLOCK_SIZE)
                .zip(padded_data.par_chunks(DES_BLOCK_SIZE))
                .for_each(|(out_slice, chunk)| {
                    CODEC.encrypt_block(chunk, out_slice);
                });
        }

        #[cfg(target_arch = "wasm32")]
        {
            encrypted_data
                .chunks_mut(DES_BLOCK_SIZE)
                .zip(padded_data.chunks(DES_BLOCK_SIZE))
                .for_each(|(out_slice, chunk)| {
                    CODEC.encrypt_block(chunk, out_slice);
                });
        }

        Ok(encode(encrypted_data))
    }

    /// 使用 Zlib 解压缩字节数据。
    /// 同时会尝试移除头部的 UTF-8 BOM (0xEF 0xBB 0xBF)。
    ///
    /// # 参数
    /// * `data` - 需要解压缩的原始字节数据。
    ///
    /// # 返回
    /// `Result<Vec<u8>, ConvertError>` - 成功时返回解压缩后的字节向量，失败时返回错误。
    fn decompress(data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| LyricsHelperError::Decryption(format!("Zlib解压缩失败: {e}")))?;

        if decompressed.starts_with(&[0xEF, 0xBB, 0xBF]) {
            decompressed.drain(..3);
        }
        Ok(decompressed)
    }

    /// 使用零字节对数据进行填充。
    ///
    /// QQ音乐使用的填充方案是零填充。
    ///
    /// # 参数
    /// * `data` - 需要填充的字节数据
    /// * `block_size` - 块大小，对于DES来说是8
    fn zero_pad(data: &[u8], block_size: usize) -> Vec<u8> {
        let padding_len = (block_size - (data.len() % block_size)) % block_size;
        if padding_len == 0 {
            return data.to_vec();
        }

        let mut padded_data = Vec::with_capacity(data.len() + padding_len);
        padded_data.extend_from_slice(data);
        padded_data.resize(data.len() + padding_len, 0);

        padded_data
    }

    /// 将所有非标准的DES实现细节移动到一个子模块中，以作清晰隔离。
    pub(crate) mod custom_des {
        use std::sync::LazyLock;

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Mode {
            Encrypt,
            Decrypt,
        }

        // 解密使用的3个8字节的DES密钥
        pub(crate) const KEY_1: &[u8; 8] = b"!@#)(*$%";
        pub(crate) const KEY_2: &[u8; 8] = b"123ZXC!@";
        pub(crate) const KEY_3: &[u8; 8] = b"!@#)(NHL";

        ////////////////////////////////////////////////////////////////////////////////////////////////////

        // --- QQ 音乐使用的非标准 S 盒定义 ---

        #[rustfmt::skip]
        const SBOX1: [u8; 64] = [
            14,  4, 13,  1,  2, 15, 11,  8,  3, 10,  6, 12,  5,  9,  0,  7,
             0, 15,  7,  4, 14,  2, 13,  1, 10,  6, 12, 11,  9,  5,  3,  8,
             4,  1, 14,  8, 13,  6,  2, 11, 15, 12,  9,  7,  3, 10,  5,  0,
            15, 12,  8,  2,  4,  9,  1,  7,  5, 11,  3, 14, 10,  0,  6, 13,
        ];

        #[rustfmt::skip]
        const SBOX2: [u8; 64] = [
            15,  1,  8, 14,  6, 11,  3,  4,  9,  7,  2, 13, 12,  0,  5, 10,
             3, 13,  4,  7, 15,  2,  8, 15, 12,  0,  1, 10,  6,  9, 11,  5,
             0, 14,  7, 11, 10,  4, 13,  1,  5,  8, 12,  6,  9,  3,  2, 15,
            13,  8, 10,  1,  3, 15,  4,  2, 11,  6,  7, 12,  0,  5, 14,  9,
        ];

        #[rustfmt::skip]
        const SBOX3: [u8; 64] = [
            10,  0,  9, 14,  6,  3, 15,  5,  1, 13, 12,  7, 11,  4,  2,  8,
            13,  7,  0,  9,  3,  4,  6, 10,  2,  8,  5, 14, 12, 11, 15,  1,
            13,  6,  4,  9,  8, 15,  3,  0, 11,  1,  2, 12,  5, 10, 14,  7,
             1, 10, 13,  0,  6,  9,  8,  7,  4, 15, 14,  3, 11,  5,  2, 12,
        ];

        #[rustfmt::skip]
        const SBOX4: [u8; 64] = [
             7, 13, 14,  3,  0,  6,  9, 10,  1,  2,  8,  5, 11, 12,  4, 15,
            13,  8, 11,  5,  6, 15,  0,  3,  4,  7,  2, 12,  1, 10, 14,  9,
            10,  6,  9,  0, 12, 11,  7, 13, 15,  1,  3, 14,  5,  2,  8,  4,
             3, 15,  0,  6, 10, 10, 13,  8,  9,  4,  5, 11, 12,  7,  2, 14,
        ];

        #[rustfmt::skip]
        const SBOX5: [u8; 64] = [
             2, 12,  4,  1,  7, 10, 11,  6,  8,  5,  3, 15, 13,  0, 14,  9,
            14, 11,  2, 12,  4,  7, 13,  1,  5,  0, 15, 10,  3,  9,  8,  6,
             4,  2,  1, 11, 10, 13,  7,  8, 15,  9, 12,  5,  6,  3,  0, 14,
            11,  8, 12,  7,  1, 14,  2, 13,  6, 15,  0,  9, 10,  4,  5,  3,
        ];

        #[rustfmt::skip]
        const SBOX6: [u8; 64] = [
            12,  1, 10, 15,  9,  2,  6,  8,  0, 13,  3,  4, 14,  7,  5, 11,
            10, 15,  4,  2,  7, 12,  9,  5,  6,  1, 13, 14,  0, 11,  3,  8,
             9, 14, 15,  5,  2,  8, 12,  3,  7,  0,  4, 10,  1, 13, 11,  6,
             4,  3,  2, 12,  9,  5, 15, 10, 11, 14,  1,  7,  6,  0,  8, 13,
        ];

        #[rustfmt::skip]
        const SBOX7: [u8; 64] = [
             4, 11,  2, 14, 15,  0,  8, 13,  3, 12,  9,  7,  5, 10,  6,  1,
            13,  0, 11,  7,  4,  9,  1, 10, 14,  3,  5, 12,  2, 15,  8,  6,
             1,  4, 11, 13, 12,  3,  7, 14, 10, 15,  6,  8,  0,  5,  9,  2,
             6, 11, 13,  8,  1,  4, 10,  7,  9,  5,  0, 15, 14,  2,  3, 12,
        ];

        #[rustfmt::skip]
        const SBOX8: [u8; 64] = [
            13,  2,  8,  4,  6, 15, 11,  1, 10,  9,  3, 14,  5,  0, 12,  7,
             1, 15, 13,  8, 10,  3,  7,  4, 12,  5,  6, 11,  0, 14,  9,  2,
             7, 11,  4,  1,  9, 12, 14,  2,  0,  6, 10, 13, 15,  3,  5,  8,
             2,  1, 14,  7,  4, 10,  8, 13, 15, 12,  9,  0,  3,  5,  6, 11,
        ];

        const S_BOXES: [[u8; 64]; 8] = [SBOX1, SBOX2, SBOX3, SBOX4, SBOX5, SBOX6, SBOX7, SBOX8];

        ////////////////////////////////////////////////////////////////////////////////////////////////////

        /// QQ 音乐使用的标准 P 盒置换规则
        #[rustfmt::skip]
        const P_BOX: [u8; 32] = [
            16,  7, 20, 21, 29, 12, 28, 17,
             1, 15, 23, 26,  5, 18, 31, 10,
             2,  8, 24, 14, 32, 27,  3,  9,
            19, 13, 30,  6, 22, 11,  4, 25,
        ];

        /// QQ 音乐使用的标准扩展置换表。
        #[rustfmt::skip]
        const E_BOX_TABLE: [u8; 48] = [
            32,  1,  2,  3,  4,  5,
             4,  5,  6,  7,  8,  9,
             8,  9, 10, 11, 12, 13,
            12, 13, 14, 15, 16, 17,
            16, 17, 18, 19, 20, 21,
            20, 21, 22, 23, 24, 25,
            24, 25, 26, 27, 28, 29,
            28, 29, 30, 31, 32,  1,
        ];

        /// 生成 S-P 盒合并查找表。
        #[allow(clippy::cast_possible_truncation)]
        fn generate_sp_tables() -> [[u32; 64]; 8] {
            let mut sp_tables = [[0u32; 64]; 8];

            for s_box_idx in 0..8 {
                for s_box_input in 0..64 {
                    let s_box_index = calculate_sbox_index(s_box_input as u8);
                    let four_bit_output = S_BOXES[s_box_idx][s_box_index];

                    let pre_p_box_val = u32::from(four_bit_output) << (28 - (s_box_idx * 4));

                    sp_tables[s_box_idx][s_box_input] =
                        apply_qq_pbox_permutation(pre_p_box_val, &P_BOX);
                }
            }
            sp_tables
        }

        /// S-P 盒合并查找表。
        static SP_TABLES: LazyLock<[[u32; 64]; 8]> = LazyLock::new(generate_sp_tables);

        ////////////////////////////////////////////////////////////////////////////////////////////////////

        /// 对一个 32 位整数应用非标准的 P 盒置换规则。
        ///
        /// # 参数
        /// * `input` - S-盒代换后的 32 位中间结果。
        /// * `table` - 定义置换规则的查找表。
        ///
        /// # 返回
        /// 经过 P-盒置换后的最终 32 位结果。
        fn apply_qq_pbox_permutation(input: u32, table: &[u8; 32]) -> u32 {
            let source_bits: [u8; 32] = std::array::from_fn(|i| ((input >> (31 - i)) & 1) as u8);
            let dest_bits: [u8; 32] = std::array::from_fn(|dest_idx| {
                let source_pos_1_based = table[dest_idx];
                source_bits[source_pos_1_based as usize - 1]
            });
            dest_bits
                .iter()
                .enumerate()
                .fold(0u32, |output, (i, &bit)| {
                    output | (u32::from(bit) << (31 - i))
                })
        }

        /// 计算 DES S-盒的查找索引。
        ///
        /// # 参数
        ///
        /// * `a`: 一个 `u8` 类型的字节。函数假定用于计算的6位数据位于此字节的低6位（从 b5 到 b0，其中 b0 是最低位）。
        const fn calculate_sbox_index(a: u8) -> usize {
            ((a & 0x20) | ((a & 0x1f) >> 1) | ((a & 0x01) << 4)) as usize
        }

        /// 对一个存储在 u32 高28位的密钥部分进行循环左移。
        const fn rotate_left_28bit_in_u32(value: u32, amount: u32) -> u32 {
            const BITS_28_MASK: u32 = 0xFFFF_FFF0;
            ((value << amount) | (value >> (28 - amount))) & BITS_28_MASK
        }

        /// 从8字节密钥中根据置换表提取位，生成一个u64。
        ///
        /// # 参数
        /// * `key` - 8字节的密钥数组。
        /// * `table` - 0-based 的位索引置换表。
        fn permute_from_key_bytes(key: [u8; 8], table: &[usize]) -> u64 {
            let word1 = u32::from_le_bytes(key[0..4].try_into().unwrap());
            let word2 = u32::from_le_bytes(key[4..8].try_into().unwrap());
            let key = (u64::from(word1) << 32) | u64::from(word2);
            let mut output = 0u64;
            let output_len = table.len();
            for (i, &pos) in table.iter().enumerate() {
                let bit = (key >> (63 - pos)) & 1;
                if bit != 0 {
                    output |= 1u64 << (output_len - 1 - i);
                }
            }
            output
        }

        /// 对一个32位整数应用 E-Box 扩展置换，生成一个48位的结果。
        ///
        /// # 参数
        /// * `input` - 32位的右半部分数据 (R_i-1)。
        ///
        /// # 返回
        /// 一个 u64，其低48位是扩展后的结果。
        fn apply_e_box_permutation(input: u32) -> u64 {
            let mut output = 0u64;
            for (i, &source_bit_pos) in E_BOX_TABLE.iter().enumerate() {
                let shift_amount = 32 - source_bit_pos;
                let bit = (input >> shift_amount) & 1;

                output |= u64::from(bit) << (47 - i);
            }
            output
        }

        /// DES 密钥调度算法。
        /// 从一个64位的主密钥（实际使用56位，每字节的最低位是奇偶校验位，被忽略）
        /// 生成16个48位的轮密钥。
        ///
        /// # 参数
        /// * `key` - 8字节的DES密钥。
        /// * `schedule` - 一个可变的二维向量，用于存储生成的16个轮密钥，每个轮密钥是6字节（48位）。
        /// * `mode` - 加密 (`Encrypt`) 或解密 (`Decrypt`) 模式。解密时轮密钥的使用顺序相反。
        #[allow(clippy::cast_possible_truncation)]
        pub(crate) fn key_schedule(key: &[u8], schedule: &mut [[u8; 6]; 16], mode: Mode) {
            // 每轮循环左移的位数表
            #[rustfmt::skip]
            const KEY_RND_SHIFT: [u32; 16] = [
                1, 1, 2, 2, 2, 2, 2, 2, 
                1, 2, 2, 2, 2, 2, 2, 1,
            ];

            // 置换选择1 (PC-1) - C部分
            #[rustfmt::skip]
            const KEY_PERM_C: [usize; 28] = [
                56, 48, 40, 32, 24, 16,  8,
                 0, 57, 49, 41, 33, 25, 17,
                 9,  1, 58, 50, 42, 34, 26,
                18, 10,  2, 59, 51, 43, 35,
            ];

            // 置换选择1 (PC-1) - D部分
            #[rustfmt::skip]
            const KEY_PERM_D: [usize; 28] = [
                62, 54, 46, 38, 30, 22, 14,
                 6, 61, 53, 45, 37, 29, 21,
                13,  5, 60, 52, 44, 36, 28,
                20, 12,  4, 27, 19, 11,  3,
            ];

            // 置换选择2 (PC-2)
            #[rustfmt::skip]
            const KEY_COMPRESSION: [usize; 48] = [
                13, 16, 10, 23,  0,  4,  2, 27,
                14,  5, 20,  9, 22, 18, 11,  3,
                25,  7, 15,  6, 26, 19, 12,  1,
                40, 51, 30, 36, 46, 54, 29, 39,
                50, 44, 32, 47, 43, 48, 38, 55,
                33, 52, 45, 41, 49, 35, 28, 31,
            ];

            let key_array: &[u8; 8] = key.try_into().expect("密钥必须是8字节");

            // 应用 PC-1
            let c0 = permute_from_key_bytes(*key_array, &KEY_PERM_C);
            let d0 = permute_from_key_bytes(*key_array, &KEY_PERM_D);

            // 将28位的结果左移4位，以匹配 `rotate_left_28bit_in_u32` 对高位对齐的期望。
            let mut c = (c0 as u32) << 4;
            let mut d = (d0 as u32) << 4;

            for (i, &shift) in KEY_RND_SHIFT.iter().enumerate() {
                c = rotate_left_28bit_in_u32(c, shift);
                d = rotate_left_28bit_in_u32(d, shift);

                let to_gen = if mode == Mode::Decrypt { 15 - i } else { i };

                let mut subkey_48bit = 0u64;

                // 应用 PC-2
                for (k, &pos) in KEY_COMPRESSION.iter().enumerate() {
                    let bit = if pos < 28 {
                        (c >> (31 - pos)) & 1
                    } else {
                        // QQ 音乐特有的怪癖，该算法的规则就是pos - 27
                        (d >> (31 - (pos - 27))) & 1
                    };

                    if bit != 0 {
                        subkey_48bit |= 1u64 << (47 - k);
                    }
                }

                let subkey_bytes = subkey_48bit.to_be_bytes();
                schedule[to_gen].copy_from_slice(&subkey_bytes[2..]);
            }
        }

        /// 存储DES置换操作的查找表
        struct DesPermutationTables {
            /// 初始置换的查找表
            ip_table: [[(u32, u32); 256]; 8],
            /// 逆初始置换的查找表
            inv_ip_table: [[u64; 256]; 8],
        }

        impl DesPermutationTables {
            /// 创建并填充所有查找表
            #[allow(clippy::cast_possible_truncation)]
            fn new() -> Self {
                /// 初始置换规则。
                #[rustfmt::skip]
                const IP_RULE: [u8; 64] = [
                    34, 42, 50, 58, 2, 10, 18, 26,
                    36, 44, 52, 60, 4, 12, 20, 28,
                    38, 46, 54, 62, 6, 14, 22, 30,
                    40, 48, 56, 64, 8, 16, 24, 32,
                    33, 41, 49, 57, 1,  9, 17, 25,
                    35, 43, 51, 59, 3, 11, 19, 27,
                    37, 45, 53, 61, 5, 13, 21, 29,
                    39, 47, 55, 63, 7, 15, 23, 31,
                ];

                /// 逆初始置换规则。
                #[rustfmt::skip]
                const INV_IP_RULE: [u8; 64] = [
                    37, 5, 45, 13, 53, 21, 61, 29,
                    38, 6, 46, 14, 54, 22, 62, 30,
                    39, 7, 47, 15, 55, 23, 63, 31,
                    40, 8, 48, 16, 56, 24, 64, 32,
                    33, 1, 41,  9, 49, 17, 57, 25,
                    34, 2, 42, 10, 50, 18, 58, 26,
                    35, 3, 43, 11, 51, 19, 59, 27,
                    36, 4, 44, 12, 52, 20, 60, 28,
                ];

                /// 使用索引表执行一次置换
                fn apply_permutation(input: [u8; 8], rule: &[u8; 64]) -> u64 {
                    let normalized_input = u64::from_be_bytes(input);
                    let mut result: u64 = 0;
                    for (i, &src_bit_pos_from_1) in rule.iter().enumerate() {
                        let src_bit_pos = src_bit_pos_from_1 as usize - 1;
                        let bit = (normalized_input >> (63 - src_bit_pos)) & 1;
                        result |= bit << (63 - i);
                    }
                    result
                }

                let mut ip_table = [[(0, 0); 256]; 8];
                let mut inv_ip_table = [[0; 256]; 8];
                let mut input = [0u8; 8];

                // 生成 IP 结果查找表
                for byte_pos in 0..8 {
                    for byte_val in 0..256 {
                        input.fill(0);
                        input[byte_pos] = byte_val as u8;
                        let permuted = apply_permutation(input, &IP_RULE);
                        ip_table[byte_pos][byte_val] = ((permuted >> 32) as u32, permuted as u32);
                    }
                }

                // 生成 InvIP 结果查找表
                for (block_pos, current_block) in inv_ip_table.iter_mut().enumerate() {
                    for (block_val, item) in current_block.iter_mut().enumerate() {
                        let temp_input_u64: u64 = (block_val as u64) << (56 - (block_pos * 8));
                        let temp_input_bytes = temp_input_u64.to_be_bytes();

                        let permuted = apply_permutation(temp_input_bytes, &INV_IP_RULE);

                        *item = permuted;
                    }
                }

                Self {
                    ip_table,
                    inv_ip_table,
                }
            }
        }

        // /// DES 的 F 函数。
        // ///
        // /// 保留一个适合阅读的版本。
        // fn f_function_readable(state: u32, key: &[u8]) -> u32 {
        //     // 使用置换表进行扩展
        //     let expanded_state = apply_e_box_permutation(state);

        //     // 将6字节的轮密钥也转换为 u64，方便进行异或
        //     let key_u64 =
        //         u64::from_be_bytes([0, 0, key[0], key[1], key[2], key[3], key[4], key[5]]);

        //     // 异或
        //     let xor_result = expanded_state ^ key_u64;

        //     // S盒代换
        //     let mut s_box_output = 0u32;

        //     for i in 0..8 {
        //         let shift_amount = 42 - (i * 6);
        //         let six_bit_chunk = ((xor_result >> shift_amount) & 0x3F) as u8;

        //         let s_box_index = calculate_sbox_index(six_bit_chunk);
        //         let four_bit_result = u32::from(S_BOXES[i][s_box_index]);

        //         s_box_output |= four_bit_result << (28 - (i * 4));
        //     }

        //     // P盒置换
        //     apply_qq_pbox_permutation(s_box_output, &P_BOX)
        // }

        /// DES 的 F 函数。
        #[rustfmt::skip]
        fn f_function(state: u32, key: &[u8]) -> u32 {
            // 扩展置换
            let expanded_state = apply_e_box_permutation(state);

            // 将6字节的轮密钥也转换为 u64，方便进行异或
            let key_u64 =
                u64::from_be_bytes([0, 0, key[0], key[1], key[2], key[3], key[4], key[5]]);

            // 异或
            let xor_result = expanded_state ^ key_u64;

            // S 盒与P 盒合并查找
            SP_TABLES[0][((xor_result >> 42) & 0x3F) as usize]
                | SP_TABLES[1][((xor_result >> 36) & 0x3F) as usize]
                | SP_TABLES[2][((xor_result >> 30) & 0x3F) as usize]
                | SP_TABLES[3][((xor_result >> 24) & 0x3F) as usize]
                | SP_TABLES[4][((xor_result >> 18) & 0x3F) as usize]
                | SP_TABLES[5][((xor_result >> 12) & 0x3F) as usize]
                | SP_TABLES[6][((xor_result >>  6) & 0x3F) as usize]
                | SP_TABLES[7][( xor_result        & 0x3F) as usize]
        }

        /// 查找表实例
        static TABLES: LazyLock<DesPermutationTables> = LazyLock::new(DesPermutationTables::new);

        /// 初始置换
        fn initial_permutation(state: &mut [u32; 2], input: &[u8]) {
            state.fill(0);
            let t = &TABLES.ip_table;
            for (t_slice, &input_byte) in t.iter().zip(input.iter()) {
                let lookup = t_slice[input_byte as usize];
                state[0] |= lookup.0;
                state[1] |= lookup.1;
            }
        }

        /// 逆初始置换
        fn inverse_permutation(state: [u32; 2], output: &mut [u8]) {
            let state_u64 = (u64::from(state[0]) << 32) | u64::from(state[1]);
            let state_bytes = state_u64.to_be_bytes();
            let result = state_bytes
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &byte)| {
                    acc | TABLES.inv_ip_table[i][byte as usize]
                });

            output.copy_from_slice(&result.to_be_bytes());
        }

        /// DES 加密/解密单个64位数据块。
        ///
        /// # 参数
        /// * `input` - 8字节的输入数据块 (明文或密文)。
        /// * `output` - 8字节的可变切片，用于存储输出数据块 (密文或明文)。
        /// * `key` - 一个包含16个轮密钥的向量的引用，每个轮密钥是6字节。
        pub(super) fn des_crypt(
            input: &[u8],
            output: &mut [u8],
            key: &[[u8; super::SUB_KEY_SIZE]; super::ROUNDS],
        ) {
            let mut state = [0u32; 2]; // 存储64位数据的左右两半 (L, R)

            // 初始置换 (IP)
            initial_permutation(&mut state, input); // state[0] = L0, state[1] = R0

            // 16轮 Feistel 网络
            // 对于前15轮，执行标准的Feistel轮：
            // L_i = R_i-1; R_i = L_i-1 XOR f(R_i-1, K_i)
            for round_key in key.iter().take(15) {
                let prev_right = state[1]; // R_i-1
                let prev_left = state[0]; // L_i-1

                // 计算新的右半部分
                state[1] = prev_left ^ f_function(prev_right, round_key); // R_i
                // 新的左半部分就是旧的右半部分
                state[0] = prev_right; // L_i
            }

            // 计算 R16 = L15 ^ f(R15, K16)，
            // 并将其结果直接与 L15 (即 state[0] 的当前值) 异或。
            // 相当于 L16 = R15, R16 = L15 ^ f(R15, K16)
            state[0] ^= f_function(state[1], &key[15]);

            // 逆初始置换
            inverse_permutation(state, output);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    const ENCRYPTED_HEX_STRING: &str =
        include_str!("../../../tests/test_data/encrypted_lyrics.hex");

    #[test]
    fn test_full_decryption_flow() {
        let decryption_result = decrypt_qrc(ENCRYPTED_HEX_STRING);

        assert!(
            decryption_result.is_ok(),
            "解密过程不应返回错误。收到的错误: {:?}",
            decryption_result.err()
        );

        let decrypted_content = decryption_result.unwrap();

        assert!(!decrypted_content.is_empty(), "解密后的内容为空字符串。");

        println!("\n✅ 解密成功！");
        println!("{decrypted_content}");
    }

    #[test]
    fn test_round_trip() {
        let initial_plaintext = decrypt_qrc(ENCRYPTED_HEX_STRING).expect("初始加密失败");

        assert!(!initial_plaintext.is_empty(), "初始解密产生了空字符串");

        let re_encrypted_hex = encrypt_qrc(&initial_plaintext).expect("再次加密失败");

        assert!(!re_encrypted_hex.is_empty(), "再次加密产生了空字符串");

        let final_plaintext = decrypt_qrc(&re_encrypted_hex).expect("最终解密失败");

        assert_eq!(initial_plaintext, final_plaintext, "初始文本不等于最终文本");

        println!("\n✅ 测试成功！初始明文与最终明文完全一致。");
    }

    #[test]
    #[ignore]
    fn capture_key_schedule() {
        let key = qrc_logic::custom_des::KEY_1;
        let mut schedule = [[0u8; 6]; 16];

        qrc_logic::custom_des::key_schedule(
            key,
            &mut schedule,
            qrc_logic::custom_des::Mode::Encrypt,
        );

        for (i, round_key) in schedule.iter().enumerate() {
            print!("[");
            for (j, byte) in round_key.iter().enumerate() {
                print!("0x{byte:02X}");
                if j < 5 {
                    print!(", ");
                }
            }
            println!("], // Round {}", i + 1);
        }
    }

    #[test]
    fn verify_key_schedule() {
        // 上面测试生成的密钥调度结果
        const ENCRYPT_SCHEDULE: [[u8; 6]; 16] = [
            [0x40, 0x0C, 0x26, 0x10, 0x28, 0x08], // Round 1
            [0x40, 0xA6, 0x20, 0x14, 0x04, 0x15], // Round 2
            [0xC0, 0x94, 0x26, 0x8B, 0x00, 0xC0], // Round 3
            [0xE0, 0x82, 0x42, 0x00, 0xE2, 0x01], // Round 4
            [0x20, 0xD2, 0x22, 0x32, 0x04, 0x04], // Round 5
            [0xA0, 0x11, 0x52, 0xC8, 0x00, 0x82], // Round 6
            [0x24, 0x42, 0x51, 0x04, 0x62, 0x09], // Round 7
            [0x07, 0x51, 0x10, 0x72, 0x10, 0x40], // Round 8
            [0x06, 0x41, 0x49, 0x4A, 0x80, 0x16], // Round 9
            [0x0B, 0x41, 0x11, 0x05, 0x44, 0x88], // Round 10
            [0x0D, 0x09, 0x89, 0x08, 0x10, 0x41], // Round 11
            [0x13, 0x20, 0x89, 0xC2, 0xC0, 0x24], // Round 12
            [0x19, 0x0C, 0x80, 0x00, 0x0E, 0x88], // Round 13
            [0x50, 0x28, 0x8C, 0x98, 0x10, 0x11], // Round 14
            [0x10, 0xA4, 0x04, 0x43, 0x42, 0x20], // Round 15
            [0xD0, 0x2C, 0x04, 0x00, 0xCA, 0x82], // Round 16
        ];

        let key = qrc_logic::custom_des::KEY_1;
        let mut schedule = [[0u8; 6]; 16];

        qrc_logic::custom_des::key_schedule(
            key,
            &mut schedule,
            qrc_logic::custom_des::Mode::Encrypt,
        );

        for i in 0..16 {
            assert_eq!(
                schedule[i],
                ENCRYPT_SCHEDULE[i],
                "轮密钥在第 {} 轮不匹配！",
                i + 1
            );
        }

        println!("✅ key_schedule 验证成功！");
    }
}
