//! Based on http://www.gc-forever.com/yagcd/chap14.html#sec14.1

use encoding_rs::{SHIFT_JIS, UTF_8};

const COLUMNS: usize = 24;
const ROWS: usize = 8;
const PIXELS_PER_COLUMN: usize = 4;
const PIXELS_PER_ROW: usize = 4;
const WIDTH: usize = 96;
const HEIGHT: usize = 32;
const UNCOMPRESSED_BYTES_PER_PIXEL: usize = 4;
const COMPRESSED_BYTES_PER_PIXEL: usize = 2;
const UNCOMPRESSED_IMAGE_SIZE: usize = WIDTH * HEIGHT * UNCOMPRESSED_BYTES_PER_PIXEL;
const COMPRESSED_IMAGE_SIZE: usize = WIDTH * HEIGHT * COMPRESSED_BYTES_PER_PIXEL;
const SHORT_TEXT_LEN: usize = 0x20;
const LONG_TEXT_LEN: usize = 0x40;
const DESCRIPTION_LEN: usize = 0x80;
const MAGIC_LEN: usize = 4;
const OFFSET_IMAGE: usize = 0x20;
const OFFSET_GAME_NAME: usize = OFFSET_IMAGE + COMPRESSED_IMAGE_SIZE;
const OFFSET_DEVELOPER_NAME: usize = OFFSET_GAME_NAME + SHORT_TEXT_LEN;
const OFFSET_FULL_GAME_NAME: usize = OFFSET_DEVELOPER_NAME + SHORT_TEXT_LEN;
const OFFSET_FULL_DEVELOPER_NAME: usize = OFFSET_FULL_GAME_NAME + LONG_TEXT_LEN;
const OFFSET_GAME_DESCRIPTION: usize = OFFSET_FULL_DEVELOPER_NAME + LONG_TEXT_LEN;
const BANNER_LEN: usize = OFFSET_GAME_DESCRIPTION + DESCRIPTION_LEN;

pub struct Banner {
    pub magic: [u8; MAGIC_LEN],
    pub image: [u8; UNCOMPRESSED_IMAGE_SIZE],
    pub game_name: String,
    pub developer_name: String,
    pub full_game_name: String,
    pub full_developer_name: String,
    pub game_description: String,
}

fn a1rgb5_to_rgba(v: &[u8]) -> [u8; 4] {
    let (x, y) = (v[0], v[1]);
    // ARRRRRGG GGGBBBBB
    let a = x >> 7;
    let r = (x >> 2) & 0b11111;
    let g = ((x & 0b11) << 3) | (y >> 5);
    let b = y & 0b11111;

    let r = (r as f32 * (255.0 / 31.0)) as u8;
    let g = (g as f32 * (255.0 / 31.0)) as u8;
    let b = (b as f32 * (255.0 / 31.0)) as u8;
    let a = a * 255;

    [r, g, b, a]
}

fn rgba_to_a1rgb5(v: &[u8]) -> [u8; 2] {
    let (r, g, b, a) = (v[0], v[1], v[2], v[3]);
    let r = (r as f32 * (31.0 / 255.0)).round() as u8;
    let g = (g as f32 * (31.0 / 255.0)).round() as u8;
    let b = (b as f32 * (31.0 / 255.0)).round() as u8;
    let a = (a >= 128) as u8;

    let x = (a << 7) | (r << 2) | (g >> 3);
    let y = (g << 5) | b;

    [x, y]
}

fn read_string(is_japanese: bool, bytes: &[u8]) -> eyre::Result<String> {
    let end = bytes.iter().position(|&x| x == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    let encoding = if is_japanese { SHIFT_JIS } else { UTF_8 };
    Ok(encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or(eyre::eyre!("Couldn't parse string"))?
        .into_owned())
}

fn write_string(is_japanese: bool, text: &str, bytes: &mut [u8]) {
    let encoding = if is_japanese { SHIFT_JIS } else { UTF_8 };
    let _ = encoding.new_encoder().encode_from_utf8(text, bytes, true);
}

impl Banner {
    pub fn parse(is_japanese: bool, data: &[u8]) -> eyre::Result<Self> {
        let mut magic = [0; MAGIC_LEN];
        magic.copy_from_slice(&data[..MAGIC_LEN]);
        if &magic != b"BNR1" && &magic != b"BNR2" {
            panic!("Invalid Banner File");
        }

        let image_data = &data[OFFSET_IMAGE..][..COMPRESSED_IMAGE_SIZE];
        let mut rgba_image = [0; UNCOMPRESSED_IMAGE_SIZE];
        let mut image_data = image_data.chunks(COMPRESSED_BYTES_PER_PIXEL);
        for row in 0..ROWS {
            let row_y = row * PIXELS_PER_ROW;
            for column in 0..COLUMNS {
                let column_x = column * PIXELS_PER_COLUMN;
                for y in 0..PIXELS_PER_ROW {
                    let y = row_y + y;
                    for x in 0..PIXELS_PER_COLUMN {
                        let x = column_x + x;
                        let pixel_index = UNCOMPRESSED_BYTES_PER_PIXEL * (y * WIDTH + x);
                        let dst = &mut rgba_image[pixel_index..][..UNCOMPRESSED_BYTES_PER_PIXEL];
                        dst.copy_from_slice(&a1rgb5_to_rgba(image_data.next().unwrap()));
                    }
                }
            }
        }

        let game_name = read_string(is_japanese, &data[OFFSET_GAME_NAME..][..SHORT_TEXT_LEN])?;
        let developer_name = read_string(
            is_japanese,
            &data[OFFSET_DEVELOPER_NAME..][..SHORT_TEXT_LEN],
        )?;
        let full_game_name =
            read_string(is_japanese, &data[OFFSET_FULL_GAME_NAME..][..LONG_TEXT_LEN])?;
        let full_developer_name = read_string(
            is_japanese,
            &data[OFFSET_FULL_DEVELOPER_NAME..][..LONG_TEXT_LEN],
        )?;
        let game_description = read_string(
            is_japanese,
            &data[OFFSET_GAME_DESCRIPTION..][..DESCRIPTION_LEN],
        )?;

        Ok(Self {
            magic,
            image: rgba_image,
            game_name,
            developer_name,
            full_game_name,
            full_developer_name,
            game_description,
        })
    }

    pub fn to_bytes(&self, is_japanese: bool) -> [u8; BANNER_LEN] {
        let mut data = [0; BANNER_LEN];

        data[..MAGIC_LEN].copy_from_slice(&self.magic);

        {
            let image_data = &mut data[OFFSET_IMAGE..][..COMPRESSED_IMAGE_SIZE];
            let mut image_data = image_data.chunks_mut(COMPRESSED_BYTES_PER_PIXEL);
            for row in 0..ROWS {
                let row_y = row * PIXELS_PER_ROW;
                for column in 0..COLUMNS {
                    let column_x = column * PIXELS_PER_COLUMN;
                    for y in 0..PIXELS_PER_ROW {
                        let y = row_y + y;
                        for x in 0..PIXELS_PER_COLUMN {
                            let x = column_x + x;
                            let pixel_index = UNCOMPRESSED_BYTES_PER_PIXEL * (y * WIDTH + x);
                            let src = &self.image[pixel_index..];
                            image_data
                                .next()
                                .unwrap()
                                .copy_from_slice(&rgba_to_a1rgb5(src));
                        }
                    }
                }
            }
        }

        write_string(
            is_japanese,
            &self.game_name,
            &mut data[OFFSET_GAME_NAME..][..SHORT_TEXT_LEN],
        );
        write_string(
            is_japanese,
            &self.developer_name,
            &mut data[OFFSET_DEVELOPER_NAME..][..SHORT_TEXT_LEN],
        );
        write_string(
            is_japanese,
            &self.full_game_name,
            &mut data[OFFSET_FULL_GAME_NAME..][..LONG_TEXT_LEN],
        );
        write_string(
            is_japanese,
            &self.full_developer_name,
            &mut data[OFFSET_FULL_DEVELOPER_NAME..][..LONG_TEXT_LEN],
        );
        write_string(
            is_japanese,
            &self.game_description,
            &mut data[OFFSET_GAME_DESCRIPTION..][..DESCRIPTION_LEN],
        );

        data
    }
}
