#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

#[cfg(not(target_arch = "wasm32"))]
use std::hint::black_box;

#[cfg(not(target_arch = "wasm32"))]
use criterion::{Criterion, criterion_group, criterion_main};

use lyrics_helper_rs::providers::qq::qrc_codec::decrypt_qrc;

const ENCRYPTED_HEX_STRING: &str = "76782DD2D2D02305BAD47017F2618CC5613974810544E844BE4D1B83CFB246AD03E593EF0A5946BC253527D85D29AA319B8ED03CFE295021A5069D9E31E738651CBA39A6B040538CC1DA3A5F4314109F3669F6E4999398B540BC91B0ED81BCB2BC59D6AD4DC6F968DDEF2BA95B1AF8CDE8D28C39B73C26E175CCD8CD0EC4F4D3D2BFD6DB8275668FA3AFCA4B468FA626AFAB16754CD8D47448A7F643E21BF0822CB3EF9EE01F64B5EC9FC4CF5AC013FE31BDE0C654B0D26C8D2DC44E07A45FDF07ED1DFA5CB5D5B03DB41F9E32DB218B2F7F09286B4AAFF42A86712D4B98BEDDD0D57227002EA453C0F1DEF24762EFBF4E398FA1D9472750CF7B469DDAF2B8B108A0D57C250FDA037FEC72C21DC70FC5C2B32DE3719BC8F75DB09C0CDB88C57DC410A728540629F4FA3A2A01DA439151C408250E96DEEDC0B7ED4D3B75BD4ABC5BC2917408B612ABD311967790A4D39BE4563E385AA1AFE23C762F68E629BC69B906AC4A6E9B732103132A3A5319C1DE2C5C3AAE7EF722080A23FEC4EB06134C926A97FC6C48B892555731CF6BADFF7AFDDD8DA1BFA5B4DD41D4D5830C38B54C7E75AC66C3B7C3367CAD9FA676BB25B142D5399909C77A48945808214DFAA9929CEB44DB6FAA0DA1186D5745A8356A94696BB83B344C9A284019F2336BC09D5A77E8C38A60A6D2236E86C3D2EDA90C2B7E91E6660BEB119B6144C8E56D020FD866263520CA4C10DC4169B3BF694FD86167294431041E91E74EECC02CF04C5E7AC6A700D9294E98EDB0B68DA234FF4C23AFEB67E6D9CF731733CCD7E2EAC70F0367C67CAF0EE95ABA6B0D8F623E98E78EA4A1F882900E63FA5436864E69F00BABD1ED9010D59BE88C40118154DCE5B8D6F5D8E7D2FB06563400ABCE4FF946557BC00B22C4C7593F328A5FDCE442A05F438015D172C0E9F56B8C8D97DEAF1DDD77518CCCB26865EFC23D48AF3CF828F5735EBB96F3FEFB526F6B83916958CAFAD4DCFFF40C3A5F2682B1D4D24033D5A88FB5E051D98540581B61B4642D9CD45C1583726ABE3FC8E160BABE053FC03C5E07E95A6ECB05BDD30DE7046CFC240FC38578CBD132BFD196F3E89203555CB8FC3870A3633D9728775BCACB9F5502E13349CFE57450919990EC9403B52BDD708DC8E064CEE9BE67EE9D07EE61E5650E721473C42875B80004B86750176B4A0683C4B240640B63AB583FEE83A2B9EA32309F6933AEA1E78975B14552AC2421F15F44F0860BAE515BE87EFBCFD27DB1ACD14EC835CAB8A27AA6C3F119BC2524AA6849D8E943D887901C652822B594134DDD019CD176B5200BB80BD7E45C1B30ACF59150F062D114D9FEB446508467F6F8906CBF30ACC8A2FB809D26BD2976A33162C0C9E23DB77F53D8B645D03129631CCA59A98BEE0467E2E9954D289CA4675776D57DC90FE494482DDD73376ABA47611E23FA72106C4320510041ECEA9120F0CAEC77F7BE1C4EF3E3CBA513F11AAB1E820E46CC7AD1A06928075EC9CDB1A4785883F21B67D6CD21B39177A7338B24A689C3690418969220794A7F8715724E920A15E2FD088D0DF6151F07916F2883AE3276AC2CCCAEA6B2B1C9654B78B8AB444DBF5A37B2FC19BFF8EADF2AE694F63836C1B00BF4A8774E45824BBF44D5C4ED2C4F6431F5D8856424525F83BF5B77DA31E6DD5C475CB860D0A8BB54D747E08448242647B236D9E87D7CAD201E22EA1BEBBE9294B5960A3E75E9847A8F4845708F5917214C2E008451C0AAA75535AD7DD59234ACDD129146A5258FEF477B730582213EF185621F0058243ACF0BD4B7138F553184ED3CEC676091E7EAE831E8C95A65D47B5A37A928C9FABEA9FA6A4F97CEBEF5066C9F418A9A31F075B4612FF820C901FACA543160DFD029B02D59EC48C771F7D685BDA7039EF02DA9198678C3E06BEC0A7672D275115C44991646035EAB4C2E4B5323B7657AC50E711D62AB3EA5ECD78B60158EAB0C164C82D314E26CD6469AB0BEE032F0FBBE3A3CD5F85497A7B193D05182EAE23AB515253B102CA7F2F0DF9DE21BF25AF34DB0772C427F03EC5067904D6D77A812B6F6A5C1F2248F362C41E419CB55C536246E9E49E702CC3EDAE7D70D683B05E01749E9B2B16C157A97CA9298CA2B55791093E5250DA6F36DBCD7C573A8E58E52ED11A12B38BD4512DBAF636D9241D6FA26BB853D259B8C4C8A41AB1603F1F367D4D7E4DEED339B1F6FE9C47A485602772B15BAE25EBED881A4ECF28327DC920C30B749527A0734482CF39851FFC9D331275262FC83E1492858AE4677A8CB80CD757BAEC25A97ED834142AF52F7289D34FE730B43C2A28CA9D93B7BFCA8FB79B665035188B07952957BE2E66BB85A9A319E0D27B3D0E9AE726213A6FC1F5B82D237669B0852DAA198CC38BFE75B1FD6B130546BFB2EB6172B4222C37B86B7427764A7C945868670186BE7B9A32C516B3DCF7432F405A70B90EE87FD1658ABD50D1037BCC8DF286D6DDC39B4E9228AF969057610722DDBAE559ACEFFC6B6CC00F503A27BCDF442EF77D79BFCC92101EA986866EEBFF3BA6C44ECFC0AA2BC1A14E20EBA58DC0CBBA3C1545D49526267865321FB1A07448C7E39A40E1E6BC5FB72860CD8B12BE10EAEBEBF6F0133FDA4CA2195E162D9367ED713F43851F8647D1665938D1B6FD4BF76E6955DDEDB2149C4127BF0BDD7CDEE2EAAF023432D33EB023D60E088CDB013F253FC6A1AC76DAD19948A4DE7C9FEB98941809B0F6183A78DCAFC8DED2635C441B415415FBEF121282756966A2C157C20B5689D139DF8DD367B68ED66E4C96726B14ABC98FA8E21F5BB30188BAEEC9947F0F072028AAEAA25DB5C0F4A4F4A922464A8422A9F167B5C5CD32CC2008D7ED4E005DC0D8032433088A019920BEAAF86E606A8256FD8170B2DFEFB5ECF8EDFC5B5A54F3391357025403828CAED8086807F140CC59A1B0EABE9B74E4579CEFEBD56881D4BAF5B1E53E070180011E583C22195128A0ABC8B2D95981370E298EA39C1A6B83A3D21EBBFFE4EBAB8DE595771B60FDB8704106112B26116E6C7F21A4551C172AB13812AE498B85B9F32CA9D6AE5DC9CE73331209D0C5E159296508089B1628867CB2882DE7E49680F17F57829478C682DACEE0C70B95E9BFCC95934F88DE5861FE5684E3D2189C8E71E5EC20A20BEF3755B8F5FC8D910B80FA9EF1393DED8289AD25C6B4EFC3B2DDB270BB16A26C92EF920DF2AF4F95B888D9B8597629B058BCC0CB87C3994B6BB2705B0072D6B8AE5D1B1A5AA64687651E9E27EDC3EF95E1FD55EB836A9BD74EC264F7496245C217899DBFE70F12C7CC2174031A7DCC51FE8B06BE0B508B59C793DBB21043850777B410921EDCCC0062926E418EAF76EA67AF847A2B58915019B72B99F71B92969D6385A780DD2A488124326E15D3E886AFA93A19CB39FF35123C1BF6524B5CB24A5FDF21A8E8F7B905BD18305DF6BD9DBFCBFCC605F0E8A1A2A6B9F493CC2AF2E13F70CA68C780E62C8BB394E530C68FD59D9B73348814049D80154C793C597775734A42991CD035CFD97460BB91A629845936B1B61F3AAABB69A0063B08A02574B13D61A89BB0156F3FAE006017B8BB0184EFDB7082AD2CFCFB68980CA14CA0E87F77124746FCBA4EA29EDF90548E56FC55C3C2156B35EE47AB9C280B9DCFEAB47A04ECAB3E457DA21718C626CD8B21C92962654D97E9CE10C638FB02481481EDD01572DCCBF327FB8A1E978B8B0EE456F718FEE6E5B6439B8E379DF485E62D2B5DAE427001D2E5BB831AE1CC176164937A966C509A616E853FE729DB6A536C069053505C195E423688DB35726506A0A60716A920C6EC7E9785C836248123327D9B3E7FD36F3CF5C774BA1D3239CD5296897E961242980468A2248A48A066354B2BF33F8ECEC1CBBC087EA051832585E67DC6D70CA83847A255B9C533D50CDB75676F0E38AFBA14A428A85D9E4FC3731706DE21989E96822A858BCA9B7948B6DF5F1263C832D247425E5A2E586B37F998DDFB7201007297DDFBCB177F9DC7D80773784E0A0F8F16F6FAF3C0BC35849DFC6ECC6197E170CBB52CB1CFFD9C142D9F9B2EE6975E6242BE633109986EF09CCB85D4BEE6E05B41A6077225182DD670B3589EA25B0068A31506FE06DB38DABDA44AC0EF7430475F455ABED8C81DDEC6135979ACFE0488E8985F3C0754651AA462A256F2AA469EF4A68DDA352C7914195FCCFF3E80618316B86AB79311DEB59C4A8B665CBA8CEA44603C56304761EF2E181124BEFDF68E661CA669A55B8A2E9118C5976E8EBFB4BC5DBCB11CF542C22A11E4BD3B5413DA570816DA280C78318103BCA762728D2BF5EC282D22C25DA688173CD5E7FCC41AFBDA4FB199337F12EEB2D89B64E35216393A95D51EE468AD950B9D9FA8B840E2497D974E5B4B315FF2413A44D4DD0127B14F491285CBAB7E4B707663B38934299BD8CE7773BB1EF7FD4082CEF4763BCFC8AA37F185680A4633A8D611BE26FAD189AD1BD86979FEEB52C5CA7F26444F03DCA21917995763E96C46104889D9B41EA9AE2D0AC7F90B43FBEF282D04AF67C2D0CF36769D35351E36E555F10ECDD4CA2D079B08084653D6593754413AD71C4668974D6E6A9A3C045A0EBD49929B94D12DBD50CAB93C24D72F259AE2571BE40EF88F1618B59EAFDBCEC79CA01797DBFC2F6A6B2A08A2140242236B76CBD2178694181F48A49A0F7B428F2EE420F8B4F62C0058F8AB3B8813A24E2A9A6E6C332CFD39F20156CA1F9C6C44DB262C62880713FE2319A82EA9439F0C1FE0504D5799AF425E9B1E5824099E61D8A84FEDD83DDB3163C920A2F88638466574AEA92AD353B00E92BEAE8678D181AF4FA52338FDCBF0CEC9A426EF4C1D9F2F4161DEDA380A800DB01884B25139AD7794C2A97CB8FBFC74099A1849D7E47A9FA71765CB3888009F4CD59D79C2923E1C32D076E1F4106D19D88737D2A4AA0BA5CE96939F73F5A3CC4AC6BBB412C3CE7441D2F421580D16BB454DFA2E4358419816448F8A7C6F092E2B9134DD8679AD6BAEEF239222F6C2D4387EAA1751AD5";

mod original {
    use flate2::read::ZlibDecoder;
    use hex::decode;
    use std::io::Read;
    use std::sync::LazyLock;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    static CIPHER: LazyLock<QqMusicCipher> = LazyLock::new(QqMusicCipher::new);
    struct QqMusicCipher {
        schedule: [Vec<Vec<u8>>; 3],
    }
    impl QqMusicCipher {
        fn new() -> Self {
            let mut schedule = [
                vec![vec![0u8; 6]; 16],
                vec![vec![0u8; 6]; 16],
                vec![vec![0u8; 6]; 16],
            ];
            custom_des::key_schedule(
                &custom_des::QQ_KEY[16..24],
                &mut schedule[0],
                custom_des::DECRYPT,
            );
            custom_des::key_schedule(
                &custom_des::QQ_KEY[8..16],
                &mut schedule[1],
                custom_des::ENCRYPT,
            );
            custom_des::key_schedule(
                &custom_des::QQ_KEY[0..8],
                &mut schedule[2],
                custom_des::DECRYPT,
            );
            Self { schedule }
        }

        fn triple_des_decrypt_block(&self, input: &[u8], output: &mut [u8]) {
            let mut temp1 = [0u8; 8];
            let mut temp2 = [0u8; 8];
            custom_des::des_crypt(input, &mut temp1, &self.schedule[0]);
            custom_des::des_crypt(&temp1, &mut temp2, &self.schedule[1]);
            custom_des::des_crypt(&temp2, output, &self.schedule[2]);
        }
    }

    pub fn decrypt_lyrics(encrypted_hex_str: &str) -> Result<String> {
        let encrypted_bytes = decode(encrypted_hex_str)?;
        if !encrypted_bytes.len().is_multiple_of(8) {
            return Err("无效的数据长度".into());
        }
        let mut decrypted_data = vec![0; encrypted_bytes.len()];

        for (i, chunk) in encrypted_bytes.chunks_exact(8).enumerate() {
            let out_slice = &mut decrypted_data[i * 8..(i + 1) * 8];
            CIPHER.triple_des_decrypt_block(chunk, out_slice);
        }

        let mut decoder = ZlibDecoder::new(decrypted_data.as_slice());
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;

        if decompressed.len() >= 3
            && decompressed[0] == 0xEF
            && decompressed[1] == 0xBB
            && decompressed[2] == 0xBF
        {
            decompressed.drain(..3);
        }

        Ok(String::from_utf8(decompressed)?)
    }

    mod custom_des {
        pub const ENCRYPT: u32 = 1;
        pub const DECRYPT: u32 = 0;
        pub const QQ_KEY: &[u8] = b"!@#)(*$%123ZXC!@!@#)(NHL";

        pub const SBOX1: [u8; 64] = [
            14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6,
            12, 11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2,
            4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
        ];
        pub const SBOX2: [u8; 64] = [
            15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0,
            1, 10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1,
            3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
        ];
        pub const SBOX3: [u8; 64] = [
            10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8,
            5, 14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13,
            0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
        ];
        pub const SBOX4: [u8; 64] = [
            7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7,
            2, 12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6,
            10, 10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
        ];
        pub const SBOX5: [u8; 64] = [
            2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0,
            15, 10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7,
            1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
        ];
        pub const SBOX6: [u8; 64] = [
            12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1,
            13, 14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12,
            9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
        ];
        pub const SBOX7: [u8; 64] = [
            4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3,
            5, 12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8,
            1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
        ];
        pub const SBOX8: [u8; 64] = [
            13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5,
            6, 11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7,
            4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
        ];

        pub const fn bit_num(a: &[u8], b: usize, c: usize) -> u32 {
            ((a[b / 32 * 4 + 3 - b % 32 / 8] >> (7 - (b % 8))) & 0x01) as u32 * (1 << c)
        }
        pub const fn extract_and_position_bit_in_byte(a: u32, b: usize, c: usize) -> u8 {
            (((a >> (31 - b)) & 0x00000001) << c) as u8
        }
        pub const fn extract_and_reposition_bit_in_word(a: u32, b: usize, c: usize) -> u32 {
            ((a << b) & 0x80000000) >> c
        }
        pub const fn sbox_bit(a: u8) -> usize {
            ((a & 0x20) | ((a & 0x1f) >> 1) | ((a & 0x01) << 4)) as usize
        }

        pub fn key_schedule(key: &[u8], schedule: &mut [Vec<u8>], mode: u32) {
            let key_rnd_shift: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
            let key_perm_c: [usize; 28] = [
                56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18,
                10, 2, 59, 51, 43, 35,
            ];
            let key_perm_d: [usize; 28] = [
                62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28,
                20, 12, 4, 27, 19, 11, 3,
            ];
            let key_compression: [usize; 48] = [
                13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12,
                1, 40, 51, 30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41,
                49, 35, 28, 31,
            ];
            let mut c = 0u32;
            let mut d = 0u32;
            for (i, &perm) in key_perm_c.iter().enumerate() {
                c |= bit_num(key, perm, 31 - i);
            }
            for (i, &perm) in key_perm_d.iter().enumerate() {
                d |= bit_num(key, perm, 31 - i);
            }
            for (i, &shift) in key_rnd_shift.iter().enumerate() {
                c = ((c << shift as usize) | (c >> (28 - shift as usize))) & 0xfffffff0;
                d = ((d << shift as usize) | (d >> (28 - shift as usize))) & 0xfffffff0;
                let to_gen = if mode == DECRYPT { 15 - i } else { i };
                for j in 0..6 {
                    schedule[to_gen][j] = 0;
                }
                for (j, &comp) in key_compression.iter().enumerate().take(24) {
                    schedule[to_gen][j / 8] |=
                        extract_and_position_bit_in_byte(c, comp, 7 - (j % 8));
                }
                for (j, &comp) in key_compression.iter().enumerate().skip(24) {
                    schedule[to_gen][j / 8] |=
                        extract_and_position_bit_in_byte(d, comp - 27, 7 - (j % 8));
                }
            }
        }
        pub fn initial_permutation(state: &mut [u32; 2], input: &[u8]) {
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

        pub fn inverse_permutation(state: &[u32; 2], output: &mut [u8]) {
            let b_map = [
                4, 12, 20, 28, 5, 13, 21, 29, 6, 14, 22, 30, 7, 15, 23, 31, 3, 11, 19, 27, 2, 10,
                18, 26, 1, 9, 17, 25, 0, 8, 16, 24,
            ];
            for i in 0..4 {
                output[i] = extract_and_position_bit_in_byte(state[1], b_map[i * 2], 7)
                    | extract_and_position_bit_in_byte(state[0], b_map[i * 2], 6)
                    | extract_and_position_bit_in_byte(state[1], b_map[i * 2 + 1], 5)
                    | extract_and_position_bit_in_byte(state[0], b_map[i * 2 + 1], 4)
                    | extract_and_position_bit_in_byte(state[1], b_map[16 + i * 2], 3)
                    | extract_and_position_bit_in_byte(state[0], b_map[16 + i * 2], 2)
                    | extract_and_position_bit_in_byte(state[1], b_map[16 + i * 2 + 1], 1)
                    | extract_and_position_bit_in_byte(state[0], b_map[16 + i * 2 + 1], 0);
                output[i + 4] = extract_and_position_bit_in_byte(state[1], b_map[8 + i * 2], 7)
                    | extract_and_position_bit_in_byte(state[0], b_map[8 + i * 2], 6)
                    | extract_and_position_bit_in_byte(state[1], b_map[8 + i * 2 + 1], 5)
                    | extract_and_position_bit_in_byte(state[0], b_map[8 + i * 2 + 1], 4)
                    | extract_and_position_bit_in_byte(state[1], b_map[24 + i * 2], 3)
                    | extract_and_position_bit_in_byte(state[0], b_map[24 + i * 2], 2)
                    | extract_and_position_bit_in_byte(state[1], b_map[24 + i * 2 + 1], 1)
                    | extract_and_position_bit_in_byte(state[0], b_map[24 + i * 2 + 1], 0);
            }
        }
        pub fn f_function(state: u32, key: &[u8]) -> u32 {
            let mut lrg_state = [0u8; 6];
            let t1 = extract_and_reposition_bit_in_word(state, 31, 0)
                | ((state & 0xf0000000) >> 1)
                | extract_and_reposition_bit_in_word(state, 4, 5)
                | extract_and_reposition_bit_in_word(state, 3, 6)
                | ((state & 0x0f000000) >> 3)
                | extract_and_reposition_bit_in_word(state, 8, 11)
                | extract_and_reposition_bit_in_word(state, 7, 12)
                | ((state & 0x00f00000) >> 5)
                | extract_and_reposition_bit_in_word(state, 12, 17)
                | extract_and_reposition_bit_in_word(state, 11, 18)
                | ((state & 0x000f0000) >> 7)
                | extract_and_reposition_bit_in_word(state, 16, 23);
            let t2 = extract_and_reposition_bit_in_word(state, 15, 0)
                | ((state & 0x0000f000) << 15)
                | extract_and_reposition_bit_in_word(state, 20, 5)
                | extract_and_reposition_bit_in_word(state, 19, 6)
                | ((state & 0x00000f00) << 13)
                | extract_and_reposition_bit_in_word(state, 24, 11)
                | extract_and_reposition_bit_in_word(state, 23, 12)
                | ((state & 0x000000f0) << 11)
                | extract_and_reposition_bit_in_word(state, 28, 17)
                | extract_and_reposition_bit_in_word(state, 27, 18)
                | ((state & 0x0000000f) << 9)
                | extract_and_reposition_bit_in_word(state, 0, 23);
            lrg_state[0] = ((t1 >> 24) & 0x000000ff) as u8;
            lrg_state[1] = ((t1 >> 16) & 0x000000ff) as u8;
            lrg_state[2] = ((t1 >> 8) & 0x000000ff) as u8;
            lrg_state[3] = ((t2 >> 24) & 0x000000ff) as u8;
            lrg_state[4] = ((t2 >> 16) & 0x000000ff) as u8;
            lrg_state[5] = ((t2 >> 8) & 0x000000ff) as u8;
            lrg_state[0] ^= key[0];
            lrg_state[1] ^= key[1];
            lrg_state[2] ^= key[2];
            lrg_state[3] ^= key[3];
            lrg_state[4] ^= key[4];
            lrg_state[5] ^= key[5];
            let mut result = ((SBOX1[sbox_bit(lrg_state[0] >> 2)] as u32) << 28)
                | ((SBOX2[sbox_bit(((lrg_state[0] & 0x03) << 4) | (lrg_state[1] >> 4))] as u32)
                    << 24)
                | ((SBOX3[sbox_bit(((lrg_state[1] & 0x0f) << 2) | (lrg_state[2] >> 6))] as u32)
                    << 20)
                | ((SBOX4[sbox_bit(lrg_state[2] & 0x3f)] as u32) << 16)
                | ((SBOX5[sbox_bit(lrg_state[3] >> 2)] as u32) << 12)
                | ((SBOX6[sbox_bit(((lrg_state[3] & 0x03) << 4) | (lrg_state[4] >> 4))] as u32)
                    << 8)
                | ((SBOX7[sbox_bit(((lrg_state[4] & 0x0f) << 2) | (lrg_state[5] >> 6))] as u32)
                    << 4)
                | (SBOX8[sbox_bit(lrg_state[5] & 0x3f)] as u32);
            result = extract_and_reposition_bit_in_word(result, 15, 0)
                | extract_and_reposition_bit_in_word(result, 6, 1)
                | extract_and_reposition_bit_in_word(result, 19, 2)
                | extract_and_reposition_bit_in_word(result, 20, 3)
                | extract_and_reposition_bit_in_word(result, 28, 4)
                | extract_and_reposition_bit_in_word(result, 11, 5)
                | extract_and_reposition_bit_in_word(result, 27, 6)
                | extract_and_reposition_bit_in_word(result, 16, 7)
                | extract_and_reposition_bit_in_word(result, 0, 8)
                | extract_and_reposition_bit_in_word(result, 14, 9)
                | extract_and_reposition_bit_in_word(result, 22, 10)
                | extract_and_reposition_bit_in_word(result, 25, 11)
                | extract_and_reposition_bit_in_word(result, 4, 12)
                | extract_and_reposition_bit_in_word(result, 17, 13)
                | extract_and_reposition_bit_in_word(result, 30, 14)
                | extract_and_reposition_bit_in_word(result, 9, 15)
                | extract_and_reposition_bit_in_word(result, 1, 16)
                | extract_and_reposition_bit_in_word(result, 7, 17)
                | extract_and_reposition_bit_in_word(result, 23, 18)
                | extract_and_reposition_bit_in_word(result, 13, 19)
                | extract_and_reposition_bit_in_word(result, 31, 20)
                | extract_and_reposition_bit_in_word(result, 26, 21)
                | extract_and_reposition_bit_in_word(result, 2, 22)
                | extract_and_reposition_bit_in_word(result, 8, 23)
                | extract_and_reposition_bit_in_word(result, 18, 24)
                | extract_and_reposition_bit_in_word(result, 12, 25)
                | extract_and_reposition_bit_in_word(result, 29, 26)
                | extract_and_reposition_bit_in_word(result, 5, 27)
                | extract_and_reposition_bit_in_word(result, 21, 28)
                | extract_and_reposition_bit_in_word(result, 10, 29)
                | extract_and_reposition_bit_in_word(result, 3, 30)
                | extract_and_reposition_bit_in_word(result, 24, 31);
            result
        }
        pub fn des_crypt(input: &[u8], output: &mut [u8], key: &[Vec<u8>]) {
            let mut state = [0u32; 2];
            initial_permutation(&mut state, input);
            for key_item in key.iter().take(15) {
                let t = state[1];
                state[1] = f_function(state[1], key_item) ^ state[0];
                state[0] = t;
            }
            state[0] ^= f_function(state[1], &key[15]);
            inverse_permutation(&state, output);
        }
    }
}

fn bench_decryption(c: &mut Criterion) {
    let mut group = c.benchmark_group("QRC Decryption Performance");

    group.bench_function("Optimized", |b| {
        b.iter(|| {
            let _ = decrypt_qrc(black_box(ENCRYPTED_HEX_STRING));
        });
    });

    group.bench_function("C/C# Original", |b| {
        b.iter(|| {
            let _ = original::decrypt_lyrics(black_box(ENCRYPTED_HEX_STRING));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_decryption);

criterion_main!(benches);
