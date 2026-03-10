use core::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub const BLOCK_SIZE: usize = 512;
pub const MAX_PAYLOAD_SIZE: usize = 476;
pub const CHECKSUM_SIZE: usize = 24;
pub const MAX_PAYLOAD_SIZE_WITH_CHECKSUM: usize =
    MAX_PAYLOAD_SIZE - CHECKSUM_SIZE;

const PADDING_BYTE: u8 = 0xFF;

/// Align to 4 byte boundary
const ALIGN: usize = 4;

/// Magic numbers: Start+Second, EndOfBlock
pub const MAGIC_NUMBER: [u32; 3] = [0x0A324655, 0x9E5D5157, 0x0AB16F30];

/// Block error kind.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub enum BlockError {
    /// There was an issue with the input buffer size or alignment
    InputBuffer,
    /// The block is corrupted (in general)
    BlockNoIncorrect,
    /// One or more of the magic numbers were incorrect
    MagicNumber,
    /// Payload size too large
    PayloadSize,
    /// No checksum provided
    NoChecksum,
    /// Checksum mismatch
    ChecksumMismatch,
    /// Checksum exceeds block size
    ChecksumExceedsBlock,
    /// Padding error
    PaddingError,
}

// #[coverage(off)]
impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputBuffer => write!(f, "Input buffer"),
            Self::BlockNoIncorrect => write!(f, "Block corruption"),
            Self::MagicNumber => write!(f, "Magic number incorrect"),
            Self::PayloadSize => write!(f, "Payload size too large"),
            Self::NoChecksum => write!(f, "No checksum provided"),
            Self::ChecksumMismatch => write!(f, "Checksum mismatch"),
            Self::ChecksumExceedsBlock => {
                write!(f, "Checksum exceeds block data range")
            }
            Self::PaddingError => write!(f, "Padding error"),
        }
    }
}

/// Block structure.
///
/// Length is fixed at 512 bytes with a variable size data section up to 476 bytes.
#[derive(Debug, Copy, Clone, Immutable, KnownLayout, FromBytes, IntoBytes)]
#[repr(C)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub struct Block {
    /// First magic number
    magic_start_0: u32,
    /// Second magic number
    magic_start_1: u32,
    /// Flags
    flags: Flags,
    /// Address in flash where the data should be written
    pub target_addr: u32,
    /// Number of bytes used in data
    data_len: u32,
    //// Sequential block number, starting at 0
    pub block_no: u32,
    /// Total number of blocks
    pub total_blocks: u32,
    /// File size (may zero) _or_ family ID
    file_size_or_family_id: u32,
    /// Payload data, may followed by extensions and checksum aligned to end of payload
    /// each element will be aligned to 4, individual lenght will be padded
    payload: [u8; MAX_PAYLOAD_SIZE],
    /// Final magic number
    magic_end: u32,
}

const _: () = {
    // Ensure block is correct size.
    assert!(core::mem::size_of::<Block>() == BLOCK_SIZE);
};

impl Default for Block {
    fn default() -> Self {
        Self {
            magic_start_0: MAGIC_NUMBER[0],
            magic_start_1: MAGIC_NUMBER[1],
            flags: Flags::default(),
            target_addr: 0,
            data_len: 0,
            block_no: 0,
            total_blocks: 0,
            file_size_or_family_id: 0,
            payload: [PADDING_BYTE; 476],
            magic_end: MAGIC_NUMBER[2],
        }
    }
}

impl Block {
    /// Create a new block with the given data.
    ///
    /// data must be less than or equal to MAX_PAYLOAD_SIZE.
    pub fn new(
        block_no: usize,
        total_blocks: usize,
        data: &[u8],
        target_addr: usize,
    ) -> Self {
        // default with correct magic numbers
        let mut this = Self::default();

        // target flash address
        assert!(target_addr <= u32::MAX as usize);
        this.target_addr = target_addr as u32;

        // block index and total
        assert!(block_no <= u32::MAX as usize);
        assert!(block_no < total_blocks);
        this.block_no = block_no as u32;
        assert!(total_blocks <= u32::MAX as usize);
        this.total_blocks = total_blocks as u32;

        // copy over data and pad if needed
        assert!(data.len() <= MAX_PAYLOAD_SIZE);
        this.data_len = data.len() as u32;
        this.payload[..data.len()].copy_from_slice(data);
        this.payload[data.len()..].fill(PADDING_BYTE);

        this
    }

    /// Construct a [`Block`] from a slice.
    ///
    /// Returns an error if critical fields are incorrect.
    pub fn from_bytes(buf: &[u8]) -> Result<Block, BlockError> {
        let block = match Block::ref_from_bytes(buf) {
            Ok(b) => b,
            // INFO: e could be used for more detailed error
            Err(_e) => return Err(BlockError::InputBuffer),
        };

        block.verify()?;

        Ok(*block)
    }

    /// Verify the block.
    pub fn verify(&self) -> Result<(), BlockError> {
        // Magic number
        if [self.magic_start_0, self.magic_start_1, self.magic_end]
            != MAGIC_NUMBER
        {
            return Err(BlockError::MagicNumber);
        }

        // Block number & total blocks
        if self.block_no >= self.total_blocks || self.total_blocks == 0 {
            return Err(BlockError::BlockNoIncorrect);
        }

        // Payload Size, Extensions & Checksum
        let bytes_assigned = self.payload_used_bytes();
        let max_size = match self.has_checksum() {
            true => MAX_PAYLOAD_SIZE_WITH_CHECKSUM,
            false => MAX_PAYLOAD_SIZE,
        };
        if bytes_assigned > max_size {
            return Err(BlockError::PayloadSize);
        }

        Ok(())
    }

    /// Get a reference to the `data` field.
    ///
    /// Length is determined by `data_len`.
    pub fn data(&self) -> &[u8] {
        let len = self.data_len as usize;
        &self.payload[..len]
    }

    /// Write payload data to the block.
    ///
    /// There will be padding after data, this will destroy Extensions and Checksum
    pub fn set_data(&mut self, data: &[u8]) {
        assert!(data.len() <= MAX_PAYLOAD_SIZE);

        self.data_len = data.len() as u32;
        self.payload[..data.len()].copy_from_slice(data);
        // Padding NOT limited to ALIGN, BUT will overwrite Extensions and Checksum
        self.payload[data.len()..].fill(PADDING_BYTE);
        self.flags.remove(Flags::ExtensionTags);
        self.flags.remove(Flags::Checksum);
    }

    /// Returns `true` if  a FamilyId is stored
    ///
    /// The field `board_family_id_or_file_size` is used for both file size and family ID.
    pub fn has_family_id(&self) -> bool {
        self.flags.contains(Flags::FamilyId)
    }

    /// Returns the file size (or zero), or `None` if a FamilyId is stored
    ///
    /// The field `board_family_id_or_file_size` is used for both file size and family ID.
    pub fn file_size(&self) -> Option<u32> {
        match self.has_family_id() {
            true => None,
            false => Some(self.file_size_or_family_id),
        }
    }

    /// Sets the file size
    ///
    /// This will replace a stored family ID
    /// The field `board_family_id_or_file_size` is used for both file size and family ID.
    pub fn set_file_size(&mut self, size: u32) {
        self.file_size_or_family_id = size;
        self.flags.remove(Flags::FamilyId);
    }

    /// Returns the board family ID, or `None` if the file size is stored
    ///
    /// The field `board_family_id_or_file_size` is used for both file size and family ID.
    pub fn family_id(&self) -> Option<u32> {
        match self.has_family_id() {
            true => Some(self.file_size_or_family_id),
            false => None,
        }
    }

    /// Sets the board family ID
    ///
    /// This will replace a stored file size
    /// The filed `board_family_id_or_field_size` is used for both file size and family ID.
    pub fn set_family_id(&mut self, id: u32) {
        self.file_size_or_family_id = id;
        self.flags.insert(Flags::FamilyId);
    }

    pub fn has_checksum(&self) -> bool {
        self.flags.contains(Flags::Checksum)
    }

    /// Returns the checksum value, or `None` if no checksum is stored
    ///
    /// Checksum is stored at the end of the payload
    pub fn checksum(&self) -> Option<&Checksum> {
        if !self.has_checksum() {
            return None;
        }
        let len = self.payload.len();
        Checksum::ref_from_bytes(&self.payload[len - CHECKSUM_SIZE..len]).ok()
    }

    /// Write a given Checksum to the block
    ///
    /// May start_addr and length exceeds this block
    /// Returns an error if the payload size is too big, no space for checksum
    pub fn set_checksum(
        &mut self,
        checksum: &Checksum,
    ) -> Result<(), BlockError> {
        if self.payload_used_bytes() > MAX_PAYLOAD_SIZE_WITH_CHECKSUM {
            return Err(BlockError::PayloadSize);
        }

        self.flags.insert(Flags::Checksum);
        let checksum_bytes = checksum.as_bytes();

        let end_index = MAX_PAYLOAD_SIZE;
        let start_index = end_index - CHECKSUM_SIZE;
        assert!(end_index == self.payload.len());
        self.payload[start_index..end_index].copy_from_slice(checksum_bytes);
        Ok(())
    }

    /// Remove the checksum from the block
    ///
    /// This will clear the checksum flag and pad all bytes of checksum
    pub fn remove_checksum(&mut self) {
        let bytes_assigned = self.payload_used_bytes();
        self.payload[bytes_assigned..].fill(PADDING_BYTE);
        self.flags.remove(Flags::Checksum);
    }

    pub fn has_extension(&self) -> bool {
        self.flags.contains(Flags::ExtensionTags)
    }

    /// Add an extension to the blocks payload
    ///
    /// Returns an error if there is not enough space for the extension.
    ///
    /// This function will construct a Extension right into the payload.
    pub fn add_extension(
        &mut self,
        tag: ExtensionTag,
        data: &[u8],
    ) -> Result<(), BlockError> {
        let len = Extension::HEADER_SIZE + data.len();

        let start = self.payload_used_bytes().next_multiple_of(ALIGN);
        let end = start + len;

        let max_end = match self.has_checksum() {
            true => MAX_PAYLOAD_SIZE_WITH_CHECKSUM,
            false => MAX_PAYLOAD_SIZE,
        };
        if end > max_end {
            return Err(BlockError::PayloadSize);
        }

        self.flags.insert(Flags::ExtensionTags);
        // Construct the Extension directly into Block.payload
        // There is currently no way to implement Extension::new()
        // since Extension contains a [u8] of dynamic size
        // https://doc.rust-lang.org/nomicon/exotic-sizes.html
        self.payload[start] = len as u8;
        let tag_bytes = tag.to_bytes();
        self.payload[start + 1..start + 4].copy_from_slice(&tag_bytes);
        self.payload[start + Extension::HEADER_SIZE..end].copy_from_slice(data);

        Ok(())
    }

    /// Returns an extension [`Iterator`].
    pub fn extensions(&self) -> Option<Extensions<'_>> {
        if !self.has_extension() {
            return None;
        }

        let mut start = self.data_len as usize;
        start = start.next_multiple_of(ALIGN);
        let end = match self.has_checksum() {
            true => MAX_PAYLOAD_SIZE_WITH_CHECKSUM,
            false => MAX_PAYLOAD_SIZE,
        };
        Some(Extensions::from_bytes(&self.payload[start..end]))
    }

    /// get the number of bytes used in the payload (data + extensions)
    ///
    /// Checksum is not considered
    pub fn payload_used_bytes(&self) -> usize {
        let mut bytes_used = self.data_len as usize;
        bytes_used = bytes_used.next_multiple_of(ALIGN);
        if let Some(mut extensions) = self.extensions() {
            for ext in &mut extensions {
                bytes_used += ext.size();
            }
        }
        bytes_used
    }
}

/// Block flags.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Immutable, FromBytes, IntoBytes,
)]
#[repr(C)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub struct Flags(u32);

bitflags::bitflags! {
    impl Flags: u32 {
        /// The block is not the main flash image.
        const NotMainFlash = 0x00000001;
        /// The block contains a file container.
        const FileContainer = 0x00001000;
        /// The block contains a family ID.
        const FamilyId = 0x00002000;
        /// The block contains a checksum.
        const Checksum = 0x00004000;
        /// The block contains a target address.
        const ExtensionTags = 0x00008000;
        const _ = !0; // non exhaustive
    }
}

/// Extensions iterator
///
/// Use the `.next()` method to iterate through all of th extensions in the
/// current block. `.next()` will return `None` when there are no more
/// extensions left or none defined in the first place.
#[derive(Debug)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub struct Extensions<'a> {
    remaining: &'a [u8],
}

impl<'a> Extensions<'a> {
    /// Create a new extension iterator from bytes
    pub fn from_bytes(data: &'a [u8]) -> Self {
        Self { remaining: data }
    }
}

impl<'a> Iterator for Extensions<'a> {
    type Item = &'a Extension;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }

        let len = self.remaining[0];

        if len < Extension::HEADER_SIZE as u8
            || len == crate::block::PADDING_BYTE
        {
            return None;
        }

        let len = len as usize;
        if len > self.remaining.len() {
            return None;
        }

        let (current, rest) = self.remaining.split_at(len);
        let item = Extension::ref_from_bytes(current).unwrap();

        // Skip to next aligned position
        let next_start = len.next_multiple_of(ALIGN);
        self.remaining = &rest[next_start.min(rest.len())..];

        Some(item)
    }
}

/// Extension Item (read only!)
///
/// An additional piece of information which can be appended after data within
/// payload of a block
///
/// This can not be constructed directly, but only via the function `block::add_extension()`
/// Therefore, this struct is only used as a reference to the data within the payload of a block.
/// Since Extensions are stored in a row within payload data, this type is read only.

#[derive(Debug, KnownLayout, Immutable, FromBytes)]
#[repr(C)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub struct Extension {
    len: u8,
    tag: [u8; 3],
    data: [u8],
}

impl Extension {
    /// Length byte + tag bytes.
    pub const HEADER_SIZE: usize = 4;

    // constructor is provided by the function `block::add_extension()`
    // struct with dynamic sized type (DST) can not have a constructor
    // https://doc.rust-lang.org/nomicon/exotic-sizes.html
    // `pub fn new(tag: ExtensionTag, data: Vec<u8>) -> Self`` is not possible
    // since the size of the struct is not known at compile time

    /// Size of the extension including the header.
    pub fn size(&self) -> usize {
        self.len as usize
    }

    /// Tag of the extension.
    ///
    /// returns the ExtensionTag enum, even if [u8; 3] is read internally
    pub fn tag(&self) -> ExtensionTag {
        ExtensionTag::from_bytes(self.tag)
    }

    /// Data of the extension.
    ///
    /// only readable, since the data is stored in a slice internaly
    pub fn data(&self) -> &[u8] {
        self.data.get(..self.len as usize).unwrap_or(&[])
    }
}

/// Extension tag.
#[derive(Debug, PartialEq, Eq)]
#[repr(u32)]
#[non_exhaustive]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub enum ExtensionTag {
    /// UTF-8 Semantic Versioning string.
    SemverString = 0x9fc7bc,
    /// UTF-8 device description.
    DescriptionString = 0x650d9d,
    /// Page size of target device.
    TargetPageSize = 0x0be9f7,
    /// SHA-2 checksum of the firmware.
    Sha2Checksum = 0xb46db0,
    /// Device type identifier.
    DeviceTypeId = 0xc8a729,
    // Other unknown tag are valid
    #[doc(hidden)]
    __Unknown(u32),
}

impl Display for ExtensionTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ExtensionTag::SemverString => write!(f, "SemverString"),
            ExtensionTag::DescriptionString => write!(f, "DescriptionString"),
            ExtensionTag::TargetPageSize => write!(f, "TargetPageSize"),
            ExtensionTag::Sha2Checksum => write!(f, "Sha2Checksum"),
            ExtensionTag::DeviceTypeId => write!(f, "DeviceType"),
            ExtensionTag::__Unknown(value) => {
                write!(f, "Unknown({:#x})", value)
            }
        }
    }
}

impl ExtensionTag {
    pub fn to_bytes(&self) -> [u8; 3] {
        match self {
            ExtensionTag::SemverString => {
                0x9fc7bc_u32.to_le_bytes()[0..3].try_into().unwrap()
            }
            ExtensionTag::DescriptionString => {
                0x650d9d_u32.to_le_bytes()[0..3].try_into().unwrap()
            }
            ExtensionTag::TargetPageSize => {
                0x0be9f7_u32.to_le_bytes()[0..3].try_into().unwrap()
            }
            ExtensionTag::Sha2Checksum => {
                0xb46db0_u32.to_le_bytes()[0..3].try_into().unwrap()
            }
            ExtensionTag::DeviceTypeId => {
                0xc8a729_u32.to_le_bytes()[0..3].try_into().unwrap()
            }
            ExtensionTag::__Unknown(value) => {
                value.to_le_bytes()[0..3].try_into().unwrap()
            }
        }
    }

    pub fn from_bytes(bytes: [u8; 3]) -> Self {
        let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]);
        match value {
            0x9fc7bc => ExtensionTag::SemverString,
            0x650d9d => ExtensionTag::DescriptionString,
            0x0be9f7 => ExtensionTag::TargetPageSize,
            0xb46db0 => ExtensionTag::Sha2Checksum,
            0xc8a729 => ExtensionTag::DeviceTypeId,
            _ => ExtensionTag::__Unknown(value),
        }
    }
}

/// Checksum information.
///
/// This is used to allow skipping over blocks that do not need to be written
/// because the data has not changed.
#[derive(
    Debug, PartialEq, Eq, Immutable, KnownLayout, FromBytes, IntoBytes,
)]
#[repr(C)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub struct Checksum {
    start: u32,
    length: u32,
    checksum: [u8; 16],
}

const _: () = {
    // Ensure Checksum is correct size.
    assert!(core::mem::size_of::<Checksum>() == CHECKSUM_SIZE);
};

impl Checksum {
    pub fn new(start: u32, length: usize, checksum: [u8; 16]) -> Self {
        assert!(length <= u32::MAX as usize);
        let length = length as u32;
        Self {
            start,
            length,
            checksum,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::block::{
        Block, BlockError, Checksum, ExtensionTag, MAGIC_NUMBER,
        MAX_PAYLOAD_SIZE, MAX_PAYLOAD_SIZE_WITH_CHECKSUM, PADDING_BYTE,
    };
    //    use std::fs::read;
    use std::io::prelude::*;
    use zerocopy::IntoBytes;

    fn get_block_no_checksum() -> Vec<u8> {
        let mut file = std::fs::File::open("test/block_no_chksum.uf2").unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        buffer
    }

    fn get_block_checksum() -> Vec<u8> {
        let mut file = std::fs::File::open("test/block_chksum.uf2").unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        buffer
    }

    #[test]
    fn magic_number() {
        assert_eq!(MAGIC_NUMBER[0].as_bytes(), b"UF2\n");
    }

    #[test]
    fn block_new() {
        let data = vec![0; 16];
        let block = Block::new(0, 1, &data, 0x2000);

        assert_eq!(block.magic_start_0, MAGIC_NUMBER[0], "Magic number0");
        assert_eq!(block.magic_start_1, MAGIC_NUMBER[1], "Magic number1");

        assert_eq!(block.flags.bits(), 0, "No checksum");
        assert_eq!(block.has_family_id(), false, "has_family_id");
        assert_eq!(block.family_id(), None, "get family_id");
        assert_eq!(block.file_size(), Some(0), "get_file_size");
        assert_eq!(block.has_checksum(), false, "has_checksum");
        assert_eq!(block.has_extension(), false, "has_extension");

        assert_eq!(block.target_addr, 0x2000, "target_addr");
        assert_eq!(block.block_no, 0, "block_no");
        assert_eq!(block.total_blocks, 1, "total_blocks");

        assert_eq!(block.data().len(), 16, "data.len()");
        assert_eq!(block.data(), &data, "payload");

        assert_eq!(block.payload[20], PADDING_BYTE, "padding byte");

        assert_eq!(block.magic_end, MAGIC_NUMBER[2], "Magic number 2");

        assert_eq!(block.verify(), Ok(()));
    }

    #[test]
    fn block_from_bytes_ok() {
        let buffer = get_block_no_checksum();
        let block = Block::from_bytes(&buffer).unwrap();
        assert_eq!(block.block_no, 0);
        assert_eq!(block.total_blocks, 1);
        assert_eq!(block.target_addr, 0x2000);
        assert_eq!(block.data().len(), 16);
        assert_eq!(block.data(), &vec![0; 16]);
    }

    #[test]
    fn block_from_bytes_defect_input_buffer() {
        let buffer = get_block_no_checksum();
        assert!(matches!(
            Block::from_bytes(&buffer[0..511]),
            Err(BlockError::InputBuffer)
        ));
    }

    #[test]
    fn block_from_bytes_defect_magic_number() {
        let mut buffer = get_block_no_checksum();
        buffer[1] = 0x00;
        assert!(matches!(
            Block::from_bytes(&buffer),
            Err(BlockError::MagicNumber)
        ));
    }

    #[test]
    fn block_from_bytes_defect_block_no() {
        let mut buffer = get_block_no_checksum();
        buffer[20] = 0xFF;
        assert!(matches!(
            Block::from_bytes(&buffer),
            Err(BlockError::BlockNoIncorrect)
        ));
    }

    #[test]
    fn block_from_bytes_defect_payload_size() {
        let mut buffer = get_block_no_checksum();
        buffer[17] = 0xFF;
        assert!(matches!(
            Block::from_bytes(&buffer),
            Err(BlockError::PayloadSize)
        ));
    }

    #[test]
    fn block_write_data() {
        let buffer = get_block_checksum();
        let mut block = Block::from_bytes(&buffer).unwrap();
        assert_eq!(block.has_checksum(), true);
        block.set_data(&vec![0xAA; 16]);
        assert_eq!(block.data(), &vec![0xAA; 16]);
        assert_eq!(block.has_checksum(), false);
    }

    #[test]
    fn block_file_size_family_id() {
        let mut block = Block::new(0, 1, &vec![0; 16], 0x2000);
        assert_eq!(block.has_family_id(), false);
        assert_eq!(block.file_size(), Some(0), "file_size = 0");
        block.set_file_size(1024);
        assert_eq!(block.file_size(), Some(1024), "file_size = 1024");

        block.set_family_id(0x12345678);
        assert_eq!(block.has_family_id(), true, "has family_id");
        assert_eq!(block.family_id(), Some(0x12345678), "family_id");
        assert_eq!(block.file_size(), None, "file size");
    }

    #[test]
    fn block_checksum() {
        let data = vec![5, 6, 8, 9, 4, 6, 5, 8, 6, 8, 9, 4, 5, 6, 8, 9];
        let mut block = Block::new(20, 21, &data, 0x2000);
        assert_eq!(block.has_checksum(), false);
        assert_eq!(block.checksum(), None);

        let chk = Checksum::new(0x2000, 16, *md5::compute(&block.data()[..16]));
        assert_eq!(block.set_checksum(&chk), Ok(()));

        assert_eq!(block.has_checksum(), true);
        let chk2 = block.checksum().unwrap();
        assert_eq!(chk2.start, chk.start);
        assert_eq!(chk2.length, chk.length);
        assert!(chk2.checksum == chk.checksum);

        assert_eq!(block.payload[452], 0, "LSB of start address");
        assert_eq!(block.payload[453], 32, "LSB+1 of start address");
        assert_eq!(block.payload[456], 16, "length");
        assert_eq!(block.payload[475], chk.checksum[15]);

        assert_eq!(block.verify(), Ok(()));
    }

    #[test]
    fn block_checksum_payload_err() {
        let mut block = Block::new(0, 1, &vec![0_u8; MAX_PAYLOAD_SIZE], 0x2000);
        let chk = Checksum::new(0x2000, MAX_PAYLOAD_SIZE, [0; 16]);
        assert_eq!(block.set_checksum(&chk), Err(BlockError::PayloadSize));
    }

    #[test]
    fn block_checksum_remove() {
        let buffer = get_block_checksum();
        let mut block = Block::from_bytes(&buffer).unwrap();
        assert_eq!(block.has_checksum(), true);
        block.remove_checksum();
        assert_eq!(block.has_checksum(), false);
    }

    #[test]
    fn block_extensions() {
        //        let buffer = read("test/block_with_ext.uf2").unwrap();
        let buffer = get_block_no_checksum();
        let mut block = Block::from_bytes(&buffer).unwrap();
        assert_eq!(block.has_extension(), false);
        block
            .add_extension(ExtensionTag::DescriptionString, b"uf2 file crate")
            .ok();
        block
            .add_extension(ExtensionTag::SemverString, b"0.1.0")
            .ok();
        block
            .add_extension(ExtensionTag::TargetPageSize, b"0x1000")
            .ok();
        block
            .add_extension(ExtensionTag::__Unknown(0x123456), b"")
            .ok();
        block
            .add_extension(ExtensionTag::__Unknown(0x004499), &vec![128; 128])
            .ok();

        let chk = Checksum::new(
            0x2000,
            block.data().len(),
            *md5::compute(&block.data()[..block.data().len()]),
        );
        assert_eq!(block.set_checksum(&chk), Ok(()));

        assert_eq!(block.has_extension(), true);
        //        println!("Number of extensions: {}", block.extensions().len());
        if let Some(mut extensions) = block.extensions() {
            for ext in &mut extensions {
                println!(
                    "{} \"{}\"",
                    ext.tag(),
                    String::from_utf8_lossy(ext.data())
                );
            }
        }
    }

    #[test]
    fn block_checksum_and_extensions() {
        let mut block = Block::new(0, 1, &vec![0; 256], 0x2000);

        let chk = Checksum::new(0x2000, block.data_len as usize, [0xAA; 16]);
        assert_eq!(block.set_checksum(&chk), Ok(()));

        const LEN_CRIT: usize = MAX_PAYLOAD_SIZE_WITH_CHECKSUM - 256 + 8;
        assert_eq!(
            block.add_extension(
                ExtensionTag::DescriptionString,
                &[0xEE; LEN_CRIT]
            ),
            Err(BlockError::PayloadSize)
        );
    }
}
