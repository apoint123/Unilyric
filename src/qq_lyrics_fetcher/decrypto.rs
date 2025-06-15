// Copyright (c) 2025 [WXRIW]
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// 导入标准库的 Read trait，用于从流中读取数据 (例如 ZlibDecoder)
use std::io::Read;
// 导入 flate2 库的 ZlibDecoder，用于 Zlib 解压缩
use flate2::read::ZlibDecoder;
// 导入项目中定义的通用错误类型 ConvertError
use crate::types::ConvertError;

// 定义加密和解密模式的常量
pub const ENCRYPT: u32 = 1; // 加密模式
pub const DECRYPT: u32 = 0; // 解密模式

// QQ音乐歌词解密使用的固定24字节密钥 (3个8字节的DES密钥)
pub const QQ_KEY: &[u8] = b"!@#)(*$%123ZXC!@!@#)(NHL";

// --- DES S-盒 (Substitution Boxes) 定义 ---
// S-盒是 DES 算法中的核心非线性组件，每个 S-盒将6位输入映射为4位输出。
// DES 共有8个不同的 S-盒。这些常量数组定义了每个 S-盒的替换表。
// 数组索引由输入的6位决定，数组元素是对应的4位输出。

pub const SBOX1: [u8; 64] = [
    14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11,
    9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9, 1, 7, 5,
    11, 3, 14, 10, 0, 6, 13,
];
// (SBOX2 到 SBOX8 的定义与 SBOX1 结构类似)
pub const SBOX2: [u8; 64] = [
    15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1, 10,
    6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15, 4, 2,
    11, 6, 7, 12, 0, 5, 14, 9,
];
pub const SBOX3: [u8; 64] = [
    10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14,
    12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6, 9, 8, 7,
    4, 15, 14, 3, 11, 5, 2, 12,
];
pub const SBOX4: [u8; 64] = [
    7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12,
    1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 10, 13, 8,
    9, 4, 5, 11, 12, 7, 2,
    14, // 注意：这里 SBOX4 的第3行第6个元素是 10，有些DES实现可能是 1
];
pub const SBOX5: [u8; 64] = [
    2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10,
    3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14, 2, 13,
    6, 15, 0, 9, 10, 4, 5, 3,
];
pub const SBOX6: [u8; 64] = [
    12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14,
    0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5, 15, 10,
    11, 14, 1, 7, 6, 0, 8, 13,
];
pub const SBOX7: [u8; 64] = [
    4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12,
    2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4, 10, 7,
    9, 5, 0, 15, 14, 2, 3, 12,
];
pub const SBOX8: [u8; 64] = [
    13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11,
    0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10, 8, 13,
    15, 12, 9, 0, 3, 5, 6, 11,
];

// --- 位操作辅助函数 ---

/// 从字节数组 `a` 中提取指定位 `b` (0-indexed from MSB of the entire array)
/// 并将其作为权重为 `2^c` 的值。用于构建置换后的32位整数。
/// 例如，`bit_num(key, 56, 31)` 会取 `key` 字节数组中第56位（从高位开始数，0是最高位），
/// 如果该位是1，则返回 `1 << 31`，否则返回0。
pub const fn bit_num(a: &[u8], b: usize, c: usize) -> u32 {
    // a[b / 32 * 4 + 3 - b % 32 / 8]: 定位到包含第 b 位的字节。
    //   b / 32 * 4: 每32位（4字节）为一组，确定是哪组字节。
    //   + 3 - b % 32 / 8: 在这4字节组内，确定是哪个字节（从低地址到高地址是第3, 2, 1, 0个字节，对应大端）。
    // (a[...] >> (7 - (b % 8))): 在该字节内，右移到目标位成为最低位。
    //   b % 8: 目标位在该字节内的索引（0-7 from MSB）。
    //   7 - (b % 8): 计算需要右移的位数。
    // & 0x01: 取出最低位（即目标位的值，0或1）。
    // as u32 * (1 << c): 将该位的值（0或1）乘以 2^c，用于在目标32位整数的相应位置上设置该位。
    ((a[b / 32 * 4 + 3 - b % 32 / 8] >> (7 - (b % 8))) & 0x01) as u32 * (1 << c)
}

/// 从32位整数 `a` 中提取指定位 `b` (0-indexed from MSB)
/// 并将其左移 `c` 位。用于从一个32位整数构建一个字节。
/// 例如，`bit_num_intr(state[1], 7, 7)` 取 `state[1]` 的第7位，并将其放到结果字节的第7位。
pub const fn bit_num_intr(a: u32, b: usize, c: usize) -> u8 {
    // (a >> (31 - b)): 将 a 的第 b 位（从MSB数，0是最高位）移到最低位。
    // & 0x00000001: 取出最低位（即第 b 位的值）。
    // << c: 将该位的值左移 c 位，放到目标字节的相应位置。
    (((a >> (31 - b)) & 0x00000001) << c) as u8
}

/// 从32位整数 `a` 中，将其左移 `b` 位，然后取最高位 (MSB)，再将此位右移 `c` 位。
/// 这个函数名 `bit_num_intl` (int left?) 配合其实现，用途似乎是想取一个特定位放到目标数的特定位置，
/// 但其实现方式 `((a << b) & 0x80000000) >> c` 比较特殊：
/// 它先将 `a` 左移 `b` 位，然后用 `& 0x80000000` 取出此时的最高位。
/// 这意味着它实际上是在考察 `a` 原始的第 `(31-b)` 位（如果 `b < 32`）。
/// 然后将这个位（如果是1，则为 `0x80000000`，否则为 `0`）右移 `c` 位。
/// 这个函数主要用在 `f_function` 中，用于P-盒置换。
pub const fn bit_num_intl(a: u32, b: usize, c: usize) -> u32 {
    ((a << b) & 0x80000000) >> c
}

/// 计算 DES S-盒的查找索引。
///
/// 在 DES 算法中，S-盒的输入是6位数据，这6位决定了S-盒的行（2位）和列（4位）。
/// 此函数的作用就是将这6位输入转换为一个0到63之间的一维数组索引。
///
/// # 参数
///
/// * `a`: 一个 `u8` 类型的字节。函数假定用于计算的6位数据位于此字节的低6位（从 b5 到 b0，其中 b0 是最低位）。
///
/// # 索引计算规则
///
/// 输入的6位（记为 `b5 b4 b3 b2 b1 b0`）按以下规则重新组合：
/// - **行索引** 由外侧的两位 `b5` 和 `b0` 决定。
/// - **列索引** 由中间的四位 `b4 b3 b2 b1` 决定。
///
/// 最终形成的6位索引值的结构是 `b5b0 b4b3b2b1`，可用于直接在S-盒查找表（一维数组）中定位。
///
/// # 代码实现
///
/// 假设输入字节 a 的低6位是 `b5 b4 b3 b2 b1 b0`:
/// - `(a & 0x20)`: 提取 `b5`，结果为 `00b50000`。
/// - `((a & 0x1f) >> 1)`: 提取 `b4 b3 b2 b1 b0`，右移一位后得到 `0000b4b3b2b1`。
/// - `((a & 0x01) << 4)`: 提取 `b0`，左移四位后得到 `000b00000`。
///
/// 将这三部分按位或（`|`）组合，最终得到 `00b5b0b4b3b2b1`，这正是我们需要的索引值。
pub const fn sbox_bit(a: u8) -> usize {
    ((a & 0x20) | ((a & 0x1f) >> 1) | ((a & 0x01) << 4)) as usize
}

/// DES 密钥调度算法。
/// 从一个64位的主密钥（实际使用56位，每字节的最低位是奇偶校验位，被忽略）
/// 生成16个48位的轮密钥。
///
/// # Arguments
/// * `key` - 8字节的DES密钥。
/// * `schedule` - 一个可变的二维向量，用于存储生成的16个轮密钥，每个轮密钥是6字节（48位）。
/// * `mode` - 加密 (`ENCRYPT`) 或解密 (`DECRYPT`) 模式。解密时轮密钥的使用顺序相反。
pub fn key_schedule(key: &[u8], schedule: &mut [Vec<u8>], mode: u32) {
    // 每轮循环左移的位数表
    let key_rnd_shift: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
    // 置换选择1 (PC-1) 的 C部分：从56位有效密钥中选择28位形成 C0
    let key_perm_c: [usize; 28] = [
        56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2,
        59, 51, 43, 35,
    ];
    // 置换选择1 (PC-1) 的 D部分：从56位有效密钥中选择28位形成 D0
    let key_perm_d: [usize; 28] = [
        62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12,
        4, 27, 19, 11, 3,
    ];
    // 置换选择2 (PC-2)，也叫压缩置换：从每轮的 Ci 和 Di (共56位) 中选择48位形成轮密钥
    let key_compression: [usize; 48] = [
        13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40,
        51, 30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
    ];

    let mut c = 0u32; // 存储密钥的左半部分 Ci
    let mut d = 0u32; // 存储密钥的右半部分 Di

    // 1. 应用 PC-1 置换，将64位主密钥转换为56位有效密钥，并分为 C0 和 D0 (各28位)
    //    通过 bit_num 函数从原始8字节密钥 key 中按 key_perm_c/d 表选择位，并构建 C0/D0。
    for (i, &perm) in key_perm_c.iter().enumerate() {
        c |= bit_num(key, perm, 31 - i); // 构建 C0
    }
    for (i, &perm) in key_perm_d.iter().enumerate() {
        d |= bit_num(key, perm, 31 - i); // 构建 D0
    }

    // 2. 生成16轮的轮密钥
    for (i, &shift) in key_rnd_shift.iter().enumerate() {
        // 根据 key_rnd_shift 表对 Ci 和 Di 进行循环左移
        c = ((c << shift as usize) | (c >> (28 - shift as usize))) & 0xfffffff0; // 循环左移并保持高28位有效 (低4位清零，虽然标准DES中是28位寄存器)
        d = ((d << shift as usize) | (d >> (28 - shift as usize))) & 0xfffffff0; // 同上

        // 确定当前生成的轮密钥存储在 schedule 中的索引
        // 如果是解密模式，轮密钥的使用顺序与加密时相反
        let to_gen = if mode == DECRYPT { 15 - i } else { i };

        // 初始化当前轮密钥的存储空间 (6字节)
        for j in 0..6 {
            schedule[to_gen][j] = 0;
        }

        // 3. 应用 PC-2 (压缩置换) 从移位后的 Ci 和 Di 生成48位轮密钥
        //    通过 bit_num_intr 函数从 C 和 D 中按 key_compression 表选择位，并构建轮密钥。
        // 前24位来自 C
        for (j, &comp) in key_compression.iter().enumerate().take(24) {
            schedule[to_gen][j / 8] |= bit_num_intr(c, comp, 7 - (j % 8));
        }
        // 后24位来自 D (注意 comp 在 key_compression 中的索引是从28开始的，所以减去27)
        for (j, &comp) in key_compression.iter().enumerate().skip(24) {
            schedule[to_gen][j / 8] |= bit_num_intr(d, comp - 27, 7 - (j % 8));
        }
    }
}

/// DES 初始置换 (IP)。
/// 将输入的64位数据块（8字节）按照固定的IP表进行位序重排。
/// 结果存储在 `state` 数组中，`state[0]` 是重排后的左32位 (L0)，`state[1]` 是右32位 (R0)。
pub fn ip(state: &mut [u32; 2], input: &[u8]) {
    // IP表的硬编码实现，通过 bit_num 函数从 input 中选择特定的位放到 state 的目标位置。
    // 例如，input 的第57位成为 state[0] 的最高位 (第31位)，input 的第49位成为 state[0] 的次高位，以此类推。
    state[0] = bit_num(input, 57, 31)
        | bit_num(input, 49, 30)
        | bit_num(input, 41, 29)
        | bit_num(input, 33, 28)
        | bit_num(input, 25, 27)
        | bit_num(input, 17, 26)
        | bit_num(input, 9, 25)
        | bit_num(input, 1, 24)
        | bit_num(input, 59, 23)
        | bit_num(input, 51, 22)
        | bit_num(input, 43, 21)
        | bit_num(input, 35, 20)
        | bit_num(input, 27, 19)
        | bit_num(input, 19, 18)
        | bit_num(input, 11, 17)
        | bit_num(input, 3, 16)
        | bit_num(input, 61, 15)
        | bit_num(input, 53, 14)
        | bit_num(input, 45, 13)
        | bit_num(input, 37, 12)
        | bit_num(input, 29, 11)
        | bit_num(input, 21, 10)
        | bit_num(input, 13, 9)
        | bit_num(input, 5, 8)
        | bit_num(input, 63, 7)
        | bit_num(input, 55, 6)
        | bit_num(input, 47, 5)
        | bit_num(input, 39, 4)
        | bit_num(input, 31, 3)
        | bit_num(input, 23, 2)
        | bit_num(input, 15, 1)
        | bit_num(input, 7, 0);

    state[1] = bit_num(input, 56, 31)
        | bit_num(input, 48, 30)
        | bit_num(input, 40, 29)
        | bit_num(input, 32, 28)
        | bit_num(input, 24, 27)
        | bit_num(input, 16, 26)
        | bit_num(input, 8, 25)
        | bit_num(input, 0, 24)
        | bit_num(input, 58, 23)
        | bit_num(input, 50, 22)
        | bit_num(input, 42, 21)
        | bit_num(input, 34, 20)
        | bit_num(input, 26, 19)
        | bit_num(input, 18, 18)
        | bit_num(input, 10, 17)
        | bit_num(input, 2, 16)
        | bit_num(input, 60, 15)
        | bit_num(input, 52, 14)
        | bit_num(input, 44, 13)
        | bit_num(input, 36, 12)
        | bit_num(input, 28, 11)
        | bit_num(input, 20, 10)
        | bit_num(input, 12, 9)
        | bit_num(input, 4, 8)
        | bit_num(input, 62, 7)
        | bit_num(input, 54, 6)
        | bit_num(input, 46, 5)
        | bit_num(input, 38, 4)
        | bit_num(input, 30, 3)
        | bit_num(input, 22, 2)
        | bit_num(input, 14, 1)
        | bit_num(input, 6, 0);
}

/// DES 逆初始置换 (InvIP)。
/// 将经过16轮处理后的64位数据块（存储在 `state` 中）按照固定的InvIP表进行位序重排，
/// 得到最终的密文或明文块（8字节），存储在 `output` 中。
pub fn inv_ip(state: &[u32; 2], output: &mut [u8]) {
    // InvIP表的硬编码实现，通过 bit_num_intr 函数从 state[0] (L16) 和 state[1] (R16) 中选择特定的位
    // 放到 output 字节数组的目标位置。
    // 注意字节顺序，output[0] 是最高字节。
    output[3] = bit_num_intr(state[1], 7, 7)
        | bit_num_intr(state[0], 7, 6)
        | bit_num_intr(state[1], 15, 5)
        | bit_num_intr(state[0], 15, 4)
        | bit_num_intr(state[1], 23, 3)
        | bit_num_intr(state[0], 23, 2)
        | bit_num_intr(state[1], 31, 1)
        | bit_num_intr(state[0], 31, 0);
    // ... (output[2] 到 output[0], output[7] 到 output[4] 的定义类似) ...
    output[2] = bit_num_intr(state[1], 6, 7)
        | bit_num_intr(state[0], 6, 6)
        | bit_num_intr(state[1], 14, 5)
        | bit_num_intr(state[0], 14, 4)
        | bit_num_intr(state[1], 22, 3)
        | bit_num_intr(state[0], 22, 2)
        | bit_num_intr(state[1], 30, 1)
        | bit_num_intr(state[0], 30, 0);
    output[1] = bit_num_intr(state[1], 5, 7)
        | bit_num_intr(state[0], 5, 6)
        | bit_num_intr(state[1], 13, 5)
        | bit_num_intr(state[0], 13, 4)
        | bit_num_intr(state[1], 21, 3)
        | bit_num_intr(state[0], 21, 2)
        | bit_num_intr(state[1], 29, 1)
        | bit_num_intr(state[0], 29, 0);
    output[0] = bit_num_intr(state[1], 4, 7)
        | bit_num_intr(state[0], 4, 6)
        | bit_num_intr(state[1], 12, 5)
        | bit_num_intr(state[0], 12, 4)
        | bit_num_intr(state[1], 20, 3)
        | bit_num_intr(state[0], 20, 2)
        | bit_num_intr(state[1], 28, 1)
        | bit_num_intr(state[0], 28, 0);
    output[7] = bit_num_intr(state[1], 3, 7)
        | bit_num_intr(state[0], 3, 6)
        | bit_num_intr(state[1], 11, 5)
        | bit_num_intr(state[0], 11, 4)
        | bit_num_intr(state[1], 19, 3)
        | bit_num_intr(state[0], 19, 2)
        | bit_num_intr(state[1], 27, 1)
        | bit_num_intr(state[0], 27, 0);
    output[6] = bit_num_intr(state[1], 2, 7)
        | bit_num_intr(state[0], 2, 6)
        | bit_num_intr(state[1], 10, 5)
        | bit_num_intr(state[0], 10, 4)
        | bit_num_intr(state[1], 18, 3)
        | bit_num_intr(state[0], 18, 2)
        | bit_num_intr(state[1], 26, 1)
        | bit_num_intr(state[0], 26, 0);
    output[5] = bit_num_intr(state[1], 1, 7)
        | bit_num_intr(state[0], 1, 6)
        | bit_num_intr(state[1], 9, 5)
        | bit_num_intr(state[0], 9, 4)
        | bit_num_intr(state[1], 17, 3)
        | bit_num_intr(state[0], 17, 2)
        | bit_num_intr(state[1], 25, 1)
        | bit_num_intr(state[0], 25, 0);
    output[4] = bit_num_intr(state[1], 0, 7)
        | bit_num_intr(state[0], 0, 6)
        | bit_num_intr(state[1], 8, 5)
        | bit_num_intr(state[0], 8, 4)
        | bit_num_intr(state[1], 16, 3)
        | bit_num_intr(state[0], 16, 2)
        | bit_num_intr(state[1], 24, 1)
        | bit_num_intr(state[0], 24, 0);
}

/// DES 的 F-函数 (Feistel 函数)。
/// 这是 DES 轮函数的核心。它接收32位的右半部分数据 (R_i-1) 和48位的轮密钥 (K_i)，
/// 输出32位的结果。
/// 步骤：
/// 1. 扩展置换 (E): 将32位的 R_i-1 扩展为48位。
/// 2. 异或: 将扩展后的48位与48位轮密钥 K_i 进行异或。
/// 3. S-盒代换: 将异或结果分为8个6位组，每个组输入到一个对应的S-盒，输出8个4位组。
/// 4. P-盒置换 (P): 将8个4位组（共32位）按照P-盒置换表进行位序重排。
pub fn f_function(state: u32, key: &[u8]) -> u32 {
    let mut lrg_state = [0u8; 6]; // 存储扩展并与轮密钥异或后的48位数据 (6字节)

    // 1. 扩展置换 (E) 和 2. 与轮密钥异或 (XOR)
    //    这里的 t1 和 t2 的计算硬编码了扩展置换 E 的逻辑。
    //    E 将输入的32位 R 扩展为48位，方法是复制 R 中的某些位。
    //    然后，扩展后的结果（这里分两部分 t1, t2 计算，每部分24位）直接与轮密钥 key (6字节) 进行异或。
    //    异或结果存储在 lrg_state 中。
    //    (具体位操作对应标准DES的E表)
    let t1 = bit_num_intl(state, 31, 0)
        | ((state & 0xf0000000) >> 1)
        | bit_num_intl(state, 4, 5)
        | bit_num_intl(state, 3, 6)
        | ((state & 0x0f000000) >> 3)
        | bit_num_intl(state, 8, 11)
        | bit_num_intl(state, 7, 12)
        | ((state & 0x00f00000) >> 5)
        | bit_num_intl(state, 12, 17)
        | bit_num_intl(state, 11, 18)
        | ((state & 0x000f0000) >> 7)
        | bit_num_intl(state, 16, 23);

    let t2 = bit_num_intl(state, 15, 0)
        | ((state & 0x0000f000) << 15)
        | bit_num_intl(state, 20, 5)
        | bit_num_intl(state, 19, 6)
        | ((state & 0x00000f00) << 13)
        | bit_num_intl(state, 24, 11)
        | bit_num_intl(state, 23, 12)
        | ((state & 0x000000f0) << 11)
        | bit_num_intl(state, 28, 17)
        | bit_num_intl(state, 27, 18)
        | ((state & 0x0000000f) << 9)
        | bit_num_intl(state, 0, 23);

    // 将扩展和异或的结果（48位）存入 lrg_state (6字节)
    lrg_state[0] = ((t1 >> 24) & 0x000000ff) as u8;
    lrg_state[1] = ((t1 >> 16) & 0x000000ff) as u8;
    lrg_state[2] = ((t1 >> 8) & 0x000000ff) as u8;
    lrg_state[3] = ((t2 >> 24) & 0x000000ff) as u8;
    lrg_state[4] = ((t2 >> 16) & 0x000000ff) as u8;
    lrg_state[5] = ((t2 >> 8) & 0x000000ff) as u8;

    // 与轮密钥 (48位，即6字节) 进行异或
    lrg_state[0] ^= key[0];
    lrg_state[1] ^= key[1];
    lrg_state[2] ^= key[2];
    lrg_state[3] ^= key[3];
    lrg_state[4] ^= key[4];
    lrg_state[5] ^= key[5];

    // 3. S-盒代换
    //    将 lrg_state (48位) 分为8个6位组，每个组输入到对应的 S-盒 (SBOX1-SBOX8)。
    //    sbox_bit 函数用于从6位输入计算S-盒查找表的索引。
    //    每个S-盒输出4位，8个S-盒共输出32位。
    let mut result = ((SBOX1[sbox_bit(lrg_state[0] >> 2)] as u32) << 28) | // SBOX1, 输入是 lrg_state[0] 的高6位
        ((SBOX2[sbox_bit(((lrg_state[0] & 0x03) << 4) | (lrg_state[1] >> 4))] as u32) << 24) | // SBOX2, 输入是 lrg_state[0]低2位 + lrg_state[1]高4位
        ((SBOX3[sbox_bit(((lrg_state[1] & 0x0f) << 2) | (lrg_state[2] >> 6))] as u32) << 20) | // SBOX3
        ((SBOX4[sbox_bit(lrg_state[2] & 0x3f)] as u32) << 16) | // SBOX4, 输入是 lrg_state[2] 的低6位
        ((SBOX5[sbox_bit(lrg_state[3] >> 2)] as u32) << 12) | // SBOX5
        ((SBOX6[sbox_bit(((lrg_state[3] & 0x03) << 4) | (lrg_state[4] >> 4))] as u32) << 8) |  // SBOX6
        ((SBOX7[sbox_bit(((lrg_state[4] & 0x0f) << 2) | (lrg_state[5] >> 6))] as u32) << 4) |  // SBOX7
        (SBOX8[sbox_bit(lrg_state[5] & 0x3f)] as u32); // SBOX8

    // 4. P-盒置换
    //    将S-盒输出的32位结果按照固定的P-盒置换表进行位序重排。
    //    这里的 bit_num_intl 函数用于实现P-盒的位选择和放置。
    result = bit_num_intl(result, 15, 0)
        | bit_num_intl(result, 6, 1)
        | bit_num_intl(result, 19, 2)
        | bit_num_intl(result, 20, 3)
        | bit_num_intl(result, 28, 4)
        | bit_num_intl(result, 11, 5)
        | bit_num_intl(result, 27, 6)
        | bit_num_intl(result, 16, 7)
        | bit_num_intl(result, 0, 8)
        | bit_num_intl(result, 14, 9)
        | bit_num_intl(result, 22, 10)
        | bit_num_intl(result, 25, 11)
        | bit_num_intl(result, 4, 12)
        | bit_num_intl(result, 17, 13)
        | bit_num_intl(result, 30, 14)
        | bit_num_intl(result, 9, 15)
        | bit_num_intl(result, 1, 16)
        | bit_num_intl(result, 7, 17)
        | bit_num_intl(result, 23, 18)
        | bit_num_intl(result, 13, 19)
        | bit_num_intl(result, 31, 20)
        | bit_num_intl(result, 26, 21)
        | bit_num_intl(result, 2, 22)
        | bit_num_intl(result, 8, 23)
        | bit_num_intl(result, 18, 24)
        | bit_num_intl(result, 12, 25)
        | bit_num_intl(result, 29, 26)
        | bit_num_intl(result, 5, 27)
        | bit_num_intl(result, 21, 28)
        | bit_num_intl(result, 10, 29)
        | bit_num_intl(result, 3, 30)
        | bit_num_intl(result, 24, 31);

    result // 返回F函数32位输出
}

/// DES 加密/解密单个64位数据块。
///
/// # Arguments
/// * `input` - 8字节的输入数据块 (明文或密文)。
/// * `output` - 8字节的可变切片，用于存储输出数据块 (密文或明文)。
/// * `key` - 一个包含16个轮密钥的向量的引用，每个轮密钥是6字节。
pub fn des_crypt(input: &[u8], output: &mut [u8], key: &[Vec<u8>]) {
    let mut state = [0u32; 2]; // 存储64位数据的左右两半 (L, R)

    // 1. 初始置换 (IP)
    ip(&mut state, input); // state[0] = L0, state[1] = R0

    // 2. 16轮 Feistel 网络
    //    对于前15轮，执行标准的Feistel轮：
    //    L_i = R_i-1; R_i = L_i-1 XOR f(R_i-1, K_i)
    for key_item in key.iter().take(15) {
        let t = state[1]; // t (临时) = R_i-1
        state[1] = f_function(state[1], key_item) ^ state[0]; // R_i = f(R_i-1, K_i) XOR L_i-1
        state[0] = t; // L_i = R_i-1 (交换)
    }

    // 经过15轮后, state = (L15, R15)

    // 第16轮: 标准DES在最后一轮计算后不交换左右两半。
    // R16 = L15 XOR f(R15, K16)
    // L16 = R15
    // 最终送入InvIP的数据块是交换后的 (R16, L16)。
    //
    // 此实现通过一个技巧，省略了最后的显式交换：
    // state[0] (即L15) 被更新为 L15 ^ f(R15, K16)，这正是 R16。
    // state[1] (即R15) 保持不变，这正是 L16。
    state[0] ^= f_function(state[1], &key[15]);

    // 此时, state 数组的内容是 (R16, L16)，这正是InvIP所需的输入顺序。
    // 这个实现技巧在结果上与“标准16轮+最终交换”等效。

    // 3. 逆初始置换 (InvIP)
    inv_ip(&state, output);
}

/// Triple DES 密钥调度。
/// 为 Triple DES 的三个阶段（通常是 加密-解密-加密 或 解密-加密-解密）设置轮密钥。
///
/// # Arguments
/// * `key` - 24字节的 Triple DES 主密钥 (由3个8字节的DES密钥拼接而成)。
/// * `schedule` - 一个三维向量，`schedule[0]`, `schedule[1]`, `schedule[2]` 分别存储
///   三个DES阶段的16个轮密钥。
/// * `mode` - `ENCRYPT` 或 `DECRYPT`。
pub fn triple_des_key_setup(key: &[u8], schedule: &mut [Vec<Vec<u8>>], mode: u32) {
    if mode == ENCRYPT {
        // 加密模式： K1加密, K2解密, K3加密
        key_schedule(&key[0..8], &mut schedule[0], mode); // K1 用于第一阶段DES加密
        key_schedule(&key[8..16], &mut schedule[1], DECRYPT); // K2 用于第二阶段DES解密
        key_schedule(&key[16..24], &mut schedule[2], mode); // K3 用于第三阶段DES加密
    } else {
        // 解密模式：为了逆转加密操作 (E-D-E with K1,K2,K3)，
        // 我们需要执行解密-加密-解密 (D-E-D) 的操作，并使用 K3, K2, K1。
        // 我们将计算好的轮密钥按执行顺序放入 schedule[0], schedule[1], schedule[2]。

        // 第1步 (D with K3): 使用 K3 (key[16..24]) 生成解密轮密钥，存入 schedule[0]。
        key_schedule(&key[16..24], &mut schedule[0], mode);

        // 第2步 (E with K2): 使用 K2 (key[8..16]) 生成加密轮密钥，存入 schedule[1]。
        key_schedule(&key[8..16], &mut schedule[1], ENCRYPT);

        // 第3步 (D with K1): 使用 K1 (key[0..8]) 生成解密轮密钥，存入 schedule[2]。
        key_schedule(&key[0..8], &mut schedule[2], mode);
    }
}

/// Triple DES 加密/解密单个 64 位数据块。
///
/// # Arguments
/// * `input`  - 8 字节输入块
/// * `output` - 8 字节输出块，用于存储加密或解密后的结果
/// * `key`    - 由 `triple_des_key_setup` 生成的三套轮密钥，按阶段存放于 `key[0]`、`key[1]`、`key[2]`
///
/// # 实现细节
/// 本函数始终按顺序对数据执行三次 DES 操作。
///
/// **重要提示：与标准的差异**
/// 本实现的密钥使用顺序 (K1->K2->K3) 与常见标准（如 NIST SP 800-67）
/// 定义的 (K3->K2->K1) 顺序不同。因此，此代码可能无法与其他标准实现互操作。
///
/// ## 加密示例 (E-D-E 模式)
/// 如果 `triple_des_key_setup(..., ENCRYPT)` 被调用，则：
/// ```text
/// // 准备阶段 (setup)
/// key = K1 的加密轮密钥
/// key = K2 的解密轮密钥
/// key = K3 的加密轮密钥
///
/// // 执行阶段 (crypt)
/// temp1 = Encrypt(input,  key)  // E_K1(Plaintext)
/// temp2 = Decrypt(temp1,  key)  // D_K2(E_K1(P))
/// output = Encrypt(temp2, key)   // E_K3(D_K2(E_K1(P)))
/// ```
///
/// ## 解密示例 (D-E-D 模式)
/// 如果 `triple_des_key_setup(..., DECRYPT)` 被调用，则：
/// ```text
/// // 准备阶段 (setup)
/// key = K3 的解密轮密钥
/// key = K2 的加密轮密钥
/// key = K1 的解密轮密钥
///
/// // 执行阶段 (crypt)
/// temp1 = Decrypt(input,  key)  // D_K3(Ciphertext)
/// temp2 = Encrypt(temp1,  key)  // E_K2(D_K3(C))
/// output = Decrypt(temp2, key)   // D_K1(E_K2(D_K3(C)))
/// ```
pub fn triple_des_crypt(input: &[u8], output: &mut [u8], key: &[Vec<Vec<u8>>]) {
    let mut temp1 = [0u8; 8]; // 第一阶段 DES 操作结果
    let mut temp2 = [0u8; 8]; // 第二阶段 DES 操作结果

    // 按 schedule[0] → schedule[1] → schedule[2] 依次调用 DES
    des_crypt(input, &mut temp1, &key[0]);
    des_crypt(&temp1, &mut temp2, &key[1]);
    des_crypt(&temp2, output, &key[2]);
}

/// 将十六进制字符串转换为字节向量。
/// 例如 "4A4B" -> vec![0x4A, 0x4B]。
///
/// # Arguments
/// * `hex_string` - 输入的十六进制字符串。
///
/// # Returns
/// `Result<Vec<u8>, ConvertError>` - 成功时返回字节向量，失败时（如包含无效十六进制字符）返回错误。
pub fn hex_string_to_byte_array(hex_string: &str) -> Result<Vec<u8>, ConvertError> {
    // 检查字符串长度是否为偶数，因为每2个十六进制字符代表1个字节
    if hex_string.len() % 2 != 0 {
        return Err(ConvertError::InvalidHex(
            "十六进制字符串长度必须为偶数".to_string(),
        ));
    }
    (0..hex_string.len())
        .step_by(2) // 每次跳过2个字符
        .map(|i| {
            // 对每对字符进行处理
            // 从字符串中切片出2个字符
            let byte_str = &hex_string[i..i + 2];
            // 将这两个十六进制字符转换为u8字节
            u8::from_str_radix(byte_str, 16).map_err(|e| {
                ConvertError::InvalidHex(format!("无效的十六进制字节 '{byte_str}': {e}"))
            })
        })
        .collect() // 收集所有结果字节到一个 Vec<u8>
}

/// 使用 Zlib 解压缩字节数据。
/// 同时会尝试移除解压缩后数据头部的 UTF-8 BOM (0xEF 0xBB 0xBF)。
///
/// # Arguments
/// * `data` - 需要解压缩的原始字节数据。
///
/// # Returns
/// `Result<Vec<u8>, ConvertError>` - 成功时返回解压缩后的字节向量，失败时返回错误。
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, ConvertError> {
    let mut decoder = ZlibDecoder::new(data); // 创建 Zlib 解码器
    let mut decompressed = Vec::new(); // 用于存储解压缩后的数据
    // 读取所有解压缩数据到 decompressed 向量
    decoder
        .read_to_end(&mut decompressed)
        .map_err(ConvertError::Decompression)?;

    // 检查并移除 UTF-8 BOM (EF BB BF)
    if decompressed.len() >= 3
        && decompressed[0] == 0xEF
        && decompressed[1] == 0xBB
        && decompressed[2] == 0xBF
    {
        Ok(decompressed[3..].to_vec()) // 返回移除 BOM 后的数据
    } else {
        Ok(decompressed) // 如果没有 BOM，直接返回解压缩数据
    }
}

/// 解密 QQ 音乐歌词（通常是 QRC 内容）。
/// 流程：十六进制字符串 -> 字节 -> Triple DES 解密 -> Zlib 解压缩 -> UTF-8 字符串。
///
/// # Arguments
/// * `encrypted` - 经过 Base64 解码后的十六进制字符串表示的加密歌词数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 成功时返回解密并解压缩后的歌词字符串，失败时返回错误。
pub fn decrypt_lyrics(encrypted_hex_str: &str) -> Result<String, ConvertError> {
    // 1. 将十六进制字符串转换为字节数组
    let encrypted_bytes = hex_string_to_byte_array(encrypted_hex_str)?;
    let mut decrypted_data = vec![0; encrypted_bytes.len()]; // 初始化用于存储解密数据的向量

    // 2. 设置 Triple DES 密钥调度
    //    `schedule` 是一个 3x16x6 的数组，存储三组轮密钥
    let mut schedule = vec![vec![vec![0u8; 6]; 16]; 3];
    triple_des_key_setup(QQ_KEY, &mut schedule, DECRYPT); // 使用固定密钥 QQ_KEY 进行解密模式的密钥调度

    // 3. 对加密数据进行分块 Triple DES 解密
    //    DES 和 Triple DES 都是块加密算法，通常处理64位（8字节）的数据块。
    for (i, chunk) in encrypted_bytes.chunks(8).enumerate() {
        // 将加密字节按8字节分块
        if chunk.len() == 8 {
            //确保是完整的8字节块
            let mut temp_decrypted_block = [0u8; 8]; // 存储当前块的解密结果
            triple_des_crypt(chunk, &mut temp_decrypted_block, &schedule); // 执行解密

            // 将解密后的块复制到结果向量的相应位置
            let start_idx = i * 8;
            let end_idx = start_idx + 8;
            if end_idx <= decrypted_data.len() {
                decrypted_data[start_idx..end_idx].copy_from_slice(&temp_decrypted_block);
            } else {
                // 如果最后一个块不足8字节，这里可能需要特殊处理或报错
                // 但通常加密数据长度是块大小的整数倍，如果不是，可能输入数据有问题
                return Err(ConvertError::Internal(
                    "加密数据长度不是8的倍数，最后一个块处理错误".to_string(),
                ));
            }
        } else if !chunk.is_empty() {
            // 如果最后一个块不足8字节且非空，这也是一个问题
            log::warn!(
                "[QQ Decrypto] 加密数据最后一个块不足8字节，长度: {}。可能导致解密不完整。",
                chunk.len()
            );
            // 可以选择填充后解密，或者直接报错，或者尝试解密（如果算法支持）
            // 当前实现会跳过不完整的尾部块，这可能导致末尾歌词丢失。
            // 更好的做法可能是要求输入数据在解密前被正确填充。
        }
    }

    // 4. 对解密后的数据进行 Zlib 解压缩
    let decompressed_bytes = decompress(&decrypted_data)?;

    // 5. 将解压缩后的字节数据转换为 UTF-8 字符串
    String::from_utf8(decompressed_bytes).map_err(ConvertError::FromUtf8)
}
