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
    use rayon::prelude::*;
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
            let mut encrypt_schedule: TripleDesKeySchedules = [[[0; SUB_KEY_SIZE]; ROUNDS]; 3];

            let mut decrypt_schedule: TripleDesKeySchedules = [[[0; SUB_KEY_SIZE]; ROUNDS]; 3];

            // 加密流程 E(K1) -> D(K2) -> E(K3)
            custom_des::key_schedule(
                custom_des::KEY_1,
                &mut encrypt_schedule[0],
                custom_des::Mode::Encrypt,
            );
            custom_des::key_schedule(
                custom_des::KEY_2,
                &mut encrypt_schedule[1],
                custom_des::Mode::Decrypt,
            );
            custom_des::key_schedule(
                custom_des::KEY_3,
                &mut encrypt_schedule[2],
                custom_des::Mode::Encrypt,
            );

            // 解密流程 D(K3) -> E(K2) -> D(K1)
            custom_des::key_schedule(
                custom_des::KEY_3,
                &mut decrypt_schedule[0],
                custom_des::Mode::Decrypt,
            );
            custom_des::key_schedule(
                custom_des::KEY_2,
                &mut decrypt_schedule[1],
                custom_des::Mode::Encrypt,
            );
            custom_des::key_schedule(
                custom_des::KEY_1,
                &mut decrypt_schedule[2],
                custom_des::Mode::Decrypt,
            );

            Self {
                encrypt_schedule,
                decrypt_schedule,
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

        if !encrypted_bytes.len().is_multiple_of(DES_BLOCK_SIZE) {
            return Err(LyricsHelperError::Decryption(format!(
                "加密数据长度不是{DES_BLOCK_SIZE}的倍数",
            )));
        }

        let mut decrypted_data = vec![0; encrypted_bytes.len()];

        decrypted_data
            .par_chunks_mut(DES_BLOCK_SIZE)
            .zip(encrypted_bytes.par_chunks(DES_BLOCK_SIZE))
            .for_each(|(out_slice, chunk)| {
                CODEC.decrypt_block(chunk, out_slice);
            });

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

        encrypted_data
            .par_chunks_mut(DES_BLOCK_SIZE)
            .zip(padded_data.par_chunks(DES_BLOCK_SIZE))
            .for_each(|(out_slice, chunk)| {
                CODEC.encrypt_block(chunk, out_slice);
            });

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
            let mut output = 0u32;
            for (dest_bit_msb_idx, &source_bit_1_based) in table.iter().enumerate() {
                let dest_bit_mask = 1u32 << (31 - dest_bit_msb_idx);
                let source_bit_mask = 1u32 << (32 - source_bit_1_based);
                if (input & source_bit_mask) != 0 {
                    output |= dest_bit_mask;
                }
            }
            output
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
        /// 这个函数对应原始C代码中的天书BITNUM宏，模拟 QQ 音乐特有的非标准的字节序处理方式。
        /// 其将 8 字节密钥视为两个独立的、小端序的32位整数拼接而成。
        ///
        /// 例如，要读取第0位（MSB），它实际访问的是 `key[3]` 的最高位。
        /// 要读取第31位，它访问的是 `key[0]` 的最低位。
        ///
        /// # 参数
        /// * `key` - 8字节的密钥数组。
        /// * `table` - 0-based 的位索引置换表。
        fn permute_from_key_bytes(key: [u8; 8], table: &[usize]) -> u64 {
            let mut output = 0u64;
            let output_len = table.len();

            for (i, &pos) in table.iter().enumerate() {
                // 计算 pos 所在的 32 位半区，对应(b)/32
                let word_index = pos / 32;

                // 计算 pos 在其所属的 32 位半区内的偏移量，对应(b)%32
                let bit_in_word = pos % 32;

                // 计算 pos 在其所属的 4 字节块内的字节索引，对应(b)%32/8
                let byte_in_word = bit_in_word / 8;

                // 计算 pos 在其所属的字节内的比特偏移量，对应(b)%8
                let bit_in_byte = bit_in_word % 8;

                // 计算最终物理索引，对应(b)/32*4+3-(b)%32/8
                let byte_index = (word_index * 4) + (3 - byte_in_word);

                let bit = (key[byte_index] >> (7 - bit_in_byte)) & 1;

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

                /// 从字节切片中获取指定索引的位
                const fn get_bit(data: &[u8], bit_index_from_1: usize) -> u64 {
                    let bit_index = bit_index_from_1 - 1;
                    let byte_index = bit_index / 8;
                    let bit_in_byte = 7 - (bit_index % 8);
                    ((data[byte_index] >> bit_in_byte) & 1) as u64
                }

                /// 使用索引表执行一次置换
                fn apply_permutation(input: [u8; 8], rule: &[u8; 64]) -> u64 {
                    let mut result: u64 = 0;
                    for (i, &src_bit) in rule.iter().enumerate() {
                        let bit = get_bit(&input, src_bit as usize);
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
        //         let four_bit_result = S_BOXES[i][s_box_index] as u32;

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
            let t = &TABLES.inv_ip_table;
            let mut result = 0u64;
            for (i, t_slice) in t.iter().enumerate() {
                let byte_chunk =
                    (if i < 4 { state[0] } else { state[1] } >> (24 - (i % 4) * 8)) & 0xFF;
                result |= t_slice[byte_chunk as usize];
            }

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

    const ENCRYPTED_HEX_STRING: &str = "76782DD2D2D02305BAD47017F2618CC5613974810544E844BE4D1B83CFB246AD03E593EF0A5946BC253527D85D29AA319B8ED03CFE295021A5069D9E31E738651CBA39A6B040538CC1DA3A5F4314109F3669F6E4999398B540BC91B0ED81BCB2BC59D6AD4DC6F968DDEF2BA95B1AF8CDE8D28C39B73C26E175CCD8CD0EC4F4D3D2BFD6DB8275668FA3AFCA4B468FA626AFAB16754CD8D47448A7F643E21BF0822CB3EF9EE01F64B5EC9FC4CF5AC013FE31BDE0C654B0D26C8D2DC44E07A45FDF07ED1DFA5CB5D5B03DB41F9E32DB218B2F7F09286B4AAFF42A86712D4B98BEDDD0D57227002EA453C0F1DEF24762EFBF4E398FA1D9472750CF7B469DDAF2B8B108A0D57C250FDA037FEC72C21DC70FC5C2B32DE3719BC8F75DB09C0CDB88C57DC410A728540629F4FA3A2A01DA439151C408250E96DEEDC0B7ED4D3B75BD4ABC5BC2917408B612ABD311967790A4D39BE4563E385AA1AFE23C762F68E629BC69B906AC4A6E9B732103132A3A5319C1DE2C5C3AAE7EF722080A23FEC4EB06134C926A97FC6C48B892555731CF6BADFF7AFDDD8DA1BFA5B4DD41D4D5830C38B54C7E75AC66C3B7C3367CAD9FA676BB25B142D5399909C77A48945808214DFAA9929CEB44DB6FAA0DA1186D5745A8356A94696BB83B344C9A284019F2336BC09D5A77E8C38A60A6D2236E86C3D2EDA90C2B7E91E6660BEB119B6144C8E56D020FD866263520CA4C10DC4169B3BF694FD86167294431041E91E74EECC02CF04C5E7AC6A700D9294E98EDB0B68DA234FF4C23AFEB67E6D9CF731733CCD7E2EAC70F0367C67CAF0EE95ABA6B0D8F623E98E78EA4A1F882900E63FA5436864E69F00BABD1ED9010D59BE88C40118154DCE5B8D6F5D8E7D2FB06563400ABCE4FF946557BC00B22C4C7593F328A5FDCE442A05F438015D172C0E9F56B8C8D97DEAF1DDD77518CCCB26865EFC23D48AF3CF828F5735EBB96F3FEFB526F6B83916958CAFAD4DCFFF40C3A5F2682B1D4D24033D5A88FB5E051D98540581B61B4642D9CD45C1583726ABE3FC8E160BABE053FC03C5E07E95A6ECB05BDD30DE7046CFC240FC38578CBD132BFD196F3E89203555CB8FC3870A3633D9728775BCACB9F5502E13349CFE57450919990EC9403B52BDD708DC8E064CEE9BE67EE9D07EE61E5650E721473C42875B80004B86750176B4A0683C4B240640B63AB583FEE83A2B9EA32309F6933AEA1E78975B14552AC2421F15F44F0860BAE515BE87EFBCFD27DB1ACD14EC835CAB8A27AA6C3F119BC2524AA6849D8E943D887901C652822B594134DDD019CD176B5200BB80BD7E45C1B30ACF59150F062D114D9FEB446508467F6F8906CBF30ACC8A2FB809D26BD2976A33162C0C9E23DB77F53D8B645D03129631CCA59A98BEE0467E2E9954D289CA4675776D57DC90FE494482DDD73376ABA47611E23FA72106C4320510041ECEA9120F0CAEC77F7BE1C4EF3E3CBA513F11AAB1E820E46CC7AD1A06928075EC9CDB1A4785883F21B67D6CD21B39177A7338B24A689C3690418969220794A7F8715724E920A15E2FD088D0DF6151F07916F2883AE3276AC2CCCAEA6B2B1C9654B78B8AB444DBF5A37B2FC19BFF8EADF2AE694F63836C1B00BF4A8774E45824BBF44D5C4ED2C4F6431F5D8856424525F83BF5B77DA31E6DD5C475CB860D0A8BB54D747E08448242647B236D9E87D7CAD201E22EA1BEBBE9294B5960A3E75E9847A8F4845708F5917214C2E008451C0AAA75535AD7DD59234ACDD129146A5258FEF477B730582213EF185621F0058243ACF0BD4B7138F553184ED3CEC676091E7EAE831E8C95A65D47B5A37A928C9FABEA9FA6A4F97CEBEF5066C9F418A9A31F075B4612FF820C901FACA543160DFD029B02D59EC48C771F7D685BDA7039EF02DA9198678C3E06BEC0A7672D275115C44991646035EAB4C2E4B5323B7657AC50E711D62AB3EA5ECD78B60158EAB0C164C82D314E26CD6469AB0BEE032F0FBBE3A3CD5F85497A7B193D05182EAE23AB515253B102CA7F2F0DF9DE21BF25AF34DB0772C427F03EC5067904D6D77A812B6F6A5C1F2248F362C41E419CB55C536246E9E49E702CC3EDAE7D70D683B05E01749E9B2B16C157A97CA9298CA2B55791093E5250DA6F36DBCD7C573A8E58E52ED11A12B38BD4512DBAF636D9241D6FA26BB853D259B8C4C8A41AB1603F1F367D4D7E4DEED339B1F6FE9C47A485602772B15BAE25EBED881A4ECF28327DC920C30B749527A0734482CF39851FFC9D331275262FC83E1492858AE4677A8CB80CD757BAEC25A97ED834142AF52F7289D34FE730B43C2A28CA9D93B7BFCA8FB79B665035188B07952957BE2E66BB85A9A319E0D27B3D0E9AE726213A6FC1F5B82D237669B0852DAA198CC38BFE75B1FD6B130546BFB2EB6172B4222C37B86B7427764A7C945868670186BE7B9A32C516B3DCF7432F405A70B90EE87FD1658ABD50D1037BCC8DF286D6DDC39B4E9228AF969057610722DDBAE559ACEFFC6B6CC00F503A27BCDF442EF77D79BFCC92101EA986866EEBFF3BA6C44ECFC0AA2BC1A14E20EBA58DC0CBBA3C1545D49526267865321FB1A07448C7E39A40E1E6BC5FB72860CD8B12BE10EAEBEBF6F0133FDA4CA2195E162D9367ED713F43851F8647D1665938D1B6FD4BF76E6955DDEDB2149C4127BF0BDD7CDEE2EAAF023432D33EB023D60E088CDB013F253FC6A1AC76DAD19948A4DE7C9FEB98941809B0F6183A78DCAFC8DED2635C441B415415FBEF121282756966A2C157C20B5689D139DF8DD367B68ED66E4C96726B14ABC98FA8E21F5BB30188BAEEC9947F0F072028AAEAA25DB5C0F4A4F4A922464A8422A9F167B5C5CD32CC2008D7ED4E005DC0D8032433088A019920BEAAF86E606A8256FD8170B2DFEFB5ECF8EDFC5B5A54F3391357025403828CAED8086807F140CC59A1B0EABE9B74E4579CEFEBD56881D4BAF5B1E53E070180011E583C22195128A0ABC8B2D95981370E298EA39C1A6B83A3D21EBBFFE4EBAB8DE595771B60FDB8704106112B26116E6C7F21A4551C172AB13812AE498B85B9F32CA9D6AE5DC9CE73331209D0C5E159296508089B1628867CB2882DE7E49680F17F57829478C682DACEE0C70B95E9BFCC95934F88DE5861FE5684E3D2189C8E71E5EC20A20BEF3755B8F5FC8D910B80FA9EF1393DED8289AD25C6B4EFC3B2DDB270BB16A26C92EF920DF2AF4F95B888D9B8597629B058BCC0CB87C3994B6BB2705B0072D6B8AE5D1B1A5AA64687651E9E27EDC3EF95E1FD55EB836A9BD74EC264F7496245C217899DBFE70F12C7CC2174031A7DCC51FE8B06BE0B508B59C793DBB21043850777B410921EDCCC0062926E418EAF76EA67AF847A2B58915019B72B99F71B92969D6385A780DD2A488124326E15D3E886AFA93A19CB39FF35123C1BF6524B5CB24A5FDF21A8E8F7B905BD18305DF6BD9DBFCBFCC605F0E8A1A2A6B9F493CC2AF2E13F70CA68C780E62C8BB394E530C68FD59D9B73348814049D80154C793C597775734A42991CD035CFD97460BB91A629845936B1B61F3AAABB69A0063B08A02574B13D61A89BB0156F3FAE006017B8BB0184EFDB7082AD2CFCFB68980CA14CA0E87F77124746FCBA4EA29EDF90548E56FC55C3C2156B35EE47AB9C280B9DCFEAB47A04ECAB3E457DA21718C626CD8B21C92962654D97E9CE10C638FB02481481EDD01572DCCBF327FB8A1E978B8B0EE456F718FEE6E5B6439B8E379DF485E62D2B5DAE427001D2E5BB831AE1CC176164937A966C509A616E853FE729DB6A536C069053505C195E423688DB35726506A0A60716A920C6EC7E9785C836248123327D9B3E7FD36F3CF5C774BA1D3239CD5296897E961242980468A2248A48A066354B2BF33F8ECEC1CBBC087EA051832585E67DC6D70CA83847A255B9C533D50CDB75676F0E38AFBA14A428A85D9E4FC3731706DE21989E96822A858BCA9B7948B6DF5F1263C832D247425E5A2E586B37F998DDFB7201007297DDFBCB177F9DC7D80773784E0A0F8F16F6FAF3C0BC35849DFC6ECC6197E170CBB52CB1CFFD9C142D9F9B2EE6975E6242BE633109986EF09CCB85D4BEE6E05B41A6077225182DD670B3589EA25B0068A31506FE06DB38DABDA44AC0EF7430475F455ABED8C81DDEC6135979ACFE0488E8985F3C0754651AA462A256F2AA469EF4A68DDA352C7914195FCCFF3E80618316B86AB79311DEB59C4A8B665CBA8CEA44603C56304761EF2E181124BEFDF68E661CA669A55B8A2E9118C5976E8EBFB4BC5DBCB11CF542C22A11E4BD3B5413DA570816DA280C78318103BCA762728D2BF5EC282D22C25DA688173CD5E7FCC41AFBDA4FB199337F12EEB2D89B64E35216393A95D51EE468AD950B9D9FA8B840E2497D974E5B4B315FF2413A44D4DD0127B14F491285CBAB7E4B707663B38934299BD8CE7773BB1EF7FD4082CEF4763BCFC8AA37F185680A4633A8D611BE26FAD189AD1BD86979FEEB52C5CA7F26444F03DCA21917995763E96C46104889D9B41EA9AE2D0AC7F90B43FBEF282D04AF67C2D0CF36769D35351E36E555F10ECDD4CA2D079B08084653D6593754413AD71C4668974D6E6A9A3C045A0EBD49929B94D12DBD50CAB93C24D72F259AE2571BE40EF88F1618B59EAFDBCEC79CA01797DBFC2F6A6B2A08A2140242236B76CBD2178694181F48A49A0F7B428F2EE420F8B4F62C0058F8AB3B8813A24E2A9A6E6C332CFD39F20156CA1F9C6C44DB262C62880713FE2319A82EA9439F0C1FE0504D5799AF425E9B1E5824099E61D8A84FEDD83DDB3163C920A2F88638466574AEA92AD353B00E92BEAE8678D181AF4FA52338FDCBF0CEC9A426EF4C1D9F2F4161DEDA380A800DB01884B25139AD7794C2A97CB8FBFC74099A1849D7E47A9FA71765CB3888009F4CD59D79C2923E1C32D076E1F4106D19D88737D2A4AA0BA5CE96939F73F5A3CC4AC6BBB412C3CE7441D2F421580D16BB454DFA2E4358419816448F8A7C6F092E2B9134DD8679AD6BAEEF239222F6C2D4387EAA1751AD5";

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
