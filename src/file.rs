use crate::block::{
    Block, BlockError, Checksum, Extension, ExtensionTag, BLOCK_SIZE,
    MAX_PAYLOAD_SIZE, MAX_PAYLOAD_SIZE_WITH_CHECKSUM,
};
use core::fmt;
use std::io::Read;
use std::path::Path;
use zerocopy::FromBytes;

/// File error kind.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt-03", derive(defmt::Format))]
pub enum FileError {
    /// There was an issue with the input buffer size or alignment.
    InputBuffer,
    /// File size mismatch.
    FileSizeMismatch,
    /// Block corruption.
    BlockCorruption(BlockError),
    /// Block order mismatch.
    BlockOrderMismatch,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputBuffer => write!(f, "Input buffer"),
            Self::FileSizeMismatch => write!(f, "File size mismatch"),
            Self::BlockCorruption(e) => {
                write!(f, "UF2 Block corruption: {}", e)
            }
            Self::BlockOrderMismatch => {
                write!(f, "UF2 Block index or order mismatch")
            }
        }
    }
}

/// Check if the given file is a valid UF2 file
///
/// Checks file size and first block
pub fn is_uf2_file(path: &Path) -> bool {
    // File size == n * BLOCK_SIZE
    let file_size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    if !file_size.is_multiple_of(BLOCK_SIZE as u64) {
        return false;
    }

    // read first Block
    // CHECK: ? internal checks will apply (e.g. Magic Bytes)
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; BLOCK_SIZE];
    if file.read_exact(&mut buf).is_err() {
        return false;
    }
    match Block::ref_from_bytes(&buf) {
        Ok(_) => (),
        Err(_) => return false,
    }

    true
}

/// Check if the given buffer is valid UF2 data
///
/// Checks file size and first block
pub fn is_uf2_buffer(buf: &[u8]) -> bool {
    // File size == n * BLOCK_SIZE
    if !buf.len().is_multiple_of(BLOCK_SIZE) {
        return false;
    }

    let mut blocks = buf.chunks_exact(BLOCK_SIZE);
    let first_block = match blocks.next() {
        Some(b) => b,
        None => return false,
    };

    match Block::ref_from_bytes(first_block) {
        Ok(_) => (),
        Err(_) => return false,
    }

    true
}

/// UF2 file structure.
#[derive(Debug)]
pub struct Uf2File {
    blocks: Vec<Block>,
}

impl Uf2File {
    /// Create a new empty UF2 file.
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Concatenate another [`Uf2File`] to this one.
    pub fn concat(&mut self, other: &Self) {
        self.blocks.extend(other.blocks.iter().cloned());
    }

    /// Construct a [`Uf2File`] from a slice.
    ///
    /// # Examples
    /// ```
    /// use uftwo::Uf2File;
    ///
    /// let buf = std::fs::read("test/example.uf2").unwrap();
    /// let uf2_file = Uf2File::from_bytes(&buf).unwrap();
    /// ```
    ///
    /// # Errors
    /// - [`FileError::FileSizeMismatch`] if the buffer size is not a multiple of the block size.
    /// - [`FileError::BlockCorruption`] if any block in the buffer is corrupted.
    pub fn from_bytes(buf: &[u8]) -> Result<Self, FileError> {
        if !buf.len().is_multiple_of(BLOCK_SIZE) {
            return Err(FileError::FileSizeMismatch);
        }

        let mut blocks = Vec::new();

        for chunk in buf.chunks_exact(BLOCK_SIZE) {
            let block = Block::from_bytes(chunk).map_err(|e| match e {
                BlockError::InputBuffer => FileError::InputBuffer,
                BlockError::MagicNumber => FileError::BlockCorruption(e),
                BlockError::PayloadSize => FileError::BlockCorruption(e),
                BlockError::ChecksumMismatch => FileError::BlockCorruption(e),
                BlockError::ChecksumExceedsBlock => {
                    FileError::BlockCorruption(e)
                }
                BlockError::NoChecksum => unreachable!(),
                BlockError::BlockNoIncorrect => FileError::BlockCorruption(e),
                BlockError::PaddingError => FileError::BlockCorruption(e),
            })?;

            blocks.push(block);
        }

        Ok(Self { blocks })
    }

    /// Construct a [`Uf2File`] from a file.
    ///
    /// # Examples
    /// ```
    /// use std::path::Path;
    /// use uftwo::Uf2File;
    ///
    /// let uf2_file = Uf2File::from_file(Path::new("test/example.uf2")).unwrap();
    /// ```
    ///
    /// # Errors
    /// - [`FileError::InputBuffer`] if the file cannot be read or is not aligned to 512 bytes.
    /// - [`FileError::FileSizeMismatch`] if the file size is not a multiple of the block size.
    /// - [`FileError::BlockCorruption`] if any block in the file is corrupted.
    pub fn from_file(path: &Path) -> Result<Self, FileError> {
        let bytes = std::fs::read(path).map_err(|e| {
            eprintln!("Failed to read file: {}", e);
            FileError::InputBuffer
        })?;
        Self::from_bytes(&bytes[..])
    }

    /// Get payload of the UF2 file with the specified family ID.
    ///
    /// # Returns
    /// - `Some(Vec<u8>)` if the payload is not empty.
    /// - `None` if the payload is empty.
    pub fn get_payload(&self, family_id: Option<u32>) -> Option<Vec<u8>> {
        let mut payload = Vec::new();

        for block in &self.blocks {
            if let Some(id) = family_id {
                if block.family_id() != Some(id) {
                    continue;
                }
            }
            payload.extend_from_slice(block.data());
        }
        if payload.is_empty() {
            None
        } else {
            Some(payload)
        }
    }

    /// list all family IDs in the UF2 file.
    ///
    /// Returns a vector of family IDs.
    /// If there are duplicate family IDs, they are removed.
    /// If there are no family IDs, returns an empty vector.
    pub fn list_family_ids(&self) -> Vec<u32> {
        let mut family_ids = Vec::new();
        for block in &self.blocks {
            if let Some(id) = block.family_id() {
                if !family_ids.contains(&id) {
                    family_ids.push(id);
                }
            }
        }
        family_ids
    }

    /// Add payload to the UF2 file.
    pub fn add_payload(
        &mut self,
        payload: &[u8],
        family_id: Option<u32>,
    ) -> Result<(), FileError> {
        let mut offset = 0;
        let mut block_num = 0;
        let total_blocks =
            payload.len().div_ceil(MAX_PAYLOAD_SIZE_WITH_CHECKSUM);

        while offset < payload.len() {
            let chunk_size =
                std::cmp::min(MAX_PAYLOAD_SIZE, payload.len() - offset);
            let mut block = Block::new(
                block_num,
                total_blocks,
                &payload[offset..offset + chunk_size],
                offset,
            );

            if let Some(id) = family_id {
                block.set_family_id(id);
            }

            self.blocks.push(block);
            offset += chunk_size;
            block_num += 1;
        }

        Ok(())
    }

    /// Verify the integrity of the UF2 file.
    ///
    /// # Examples
    /// ```
    /// use uftwo::Uf2File;
    ///
    /// let buf = std::fs::read("test/example.uf2").unwrap();
    /// let uf2_file = Uf2File::from_bytes(&buf).unwrap();
    /// assert!(uf2_file.verify().is_ok());
    /// ```
    ///
    /// # Errors
    /// - [`FileError::BlockOrderMismatch`] if the block order is incorrect.
    /// - [`FileError::BlockCorruption`] if any block is corrupted.
    pub fn verify(&self) -> Result<(), FileError> {
        let mut prev_index = None;
        let mut prev_total_blocks = None;

        for block in &self.blocks {
            let index = block.block_no as usize;
            let total = block.total_blocks as usize;

            // block id must be < total_blocks
            if index >= total {
                return Err(FileError::BlockOrderMismatch);
            }

            // block id must be sequential, or reset to 0
            if let Some(prev_index) = prev_index {
                if index != prev_index + 1 && index != 0 {
                    return Err(FileError::BlockOrderMismatch);
                }
            }

            // total must be consistent, unless index is 0
            if let Some(prev_total_blocks) = prev_total_blocks {
                if total != prev_total_blocks && index != 0 {
                    return Err(FileError::BlockOrderMismatch);
                }
            }

            // if checksum is set, verify checksum and data length
            if block.has_checksum() {
                if block.data().len() > MAX_PAYLOAD_SIZE_WITH_CHECKSUM {
                    return Err(FileError::BlockCorruption(
                        BlockError::PayloadSize,
                    ));
                }
            } else {
                // verify data length
                if block.data().len() > MAX_PAYLOAD_SIZE {
                    return Err(FileError::BlockCorruption(
                        BlockError::PayloadSize,
                    ));
                }
            }

            prev_index = Some(index);
            prev_total_blocks = Some(total);
        }

        Ok(())
    }

    /// Create a UF2 file from binary data with extensions.
    ///
    /// Adds binary data to an existing UF2 file, converting it into UF2 blocks with checksums.
    /// Extensions (`TargetPageSize` and `SemVerString`) are added to the first block of each flash page.
    ///
    /// # Examples
    /// ```
    /// use uftwo::Uf2File;
    ///
    /// let mut uf2_file = Uf2File::new();
    /// let binary_data = vec![0xAA; 256];
    /// uf2_file.add_binary(&binary_data, 0x2000, None, 128, "1.0.0").unwrap();
    /// ```
    ///
    /// # Arguments
    /// * `binary_data` - Binary data to add to the UF2 file.
    /// * `target_addr` - Starting address for the first flash page.
    /// * `family_id` - Optional family ID for the UF2 file.
    /// * `page_size` - Size of the flash page for the target device.
    /// * `semver` - Semantic version string for the firmware.
    ///
    /// # Returns
    /// `Result<(), FileError>` - Ok if successful, Err if block creation fails.
    ///
    /// # Errors
    /// - [`FileError::BlockCorruption`] if any block cannot be created or extensions cannot be added.
    pub fn add_binary(
        &mut self,
        binary: &[u8],
        target_addr: u32,
        family_id: Option<u32>,
        page_size: usize,
        semver: &str,
    ) -> Result<(), FileError> {
        let mut new_file = Uf2File::new();
        let mut block_no = 0;
        let mut target_offset = 0;

        while target_offset < binary.len() {
            // Determine the base address for this page
            let addr = target_addr as usize + target_offset;

            // Calculate the size of the current page
            let next_page_addr = addr.next_multiple_of(page_size);
            let mut this_page_size = next_page_addr - addr;
            if this_page_size == 0 {
                this_page_size = page_size;
            }

            // Calculate how much data fits in this page
            let remaining_data = binary.len() - target_offset;
            let this_page_size = this_page_size.min(remaining_data);

            let page = &binary[target_offset..target_offset + this_page_size];

            // Calculate checksum for the entire flash page
            let checksum =
                Checksum::new(addr as u32, page.len(), *md5::compute(page));

            create_blocks_for_page(
                page,
                addr,
                family_id,
                page_size,
                semver,
                checksum,
                &mut new_file.blocks,
                &mut block_no,
            )?;

            // Advance to next page
            target_offset += this_page_size;
        }

        // Update total_blocks for all blocks (existing + new)
        let total_blocks = new_file.blocks.len();
        for block in &mut new_file.blocks {
            block.total_blocks = total_blocks as u32;
        }

        self.concat(&new_file);
        Ok(())
    }
}

/// Get blocks from a page of data
///
/// no block will have total_blocks set (placeholder only)
/// first block of page will have extensions page_size and semver
/// all blocks of page will have page_checksum
fn create_blocks_for_page(
    page_data: &[u8],
    page_addr: usize,
    family_id: Option<u32>,
    page_size: usize,
    semver: &str,
    page_checksum: Checksum,
    blocks: &mut Vec<Block>,
    block_no: &mut usize,
) -> Result<(), FileError> {
    let mut offset = 0;
    while offset < page_data.len() {
        // Limit the chunk size for the first block to leave space for extensions
        let mut max_chunk_size = std::cmp::min(
            MAX_PAYLOAD_SIZE_WITH_CHECKSUM,
            page_data.len() - offset,
        );

        if offset == 0 {
            // Calculate the size of the extensions including padding
            let target_page_size_ext = Extension::HEADER_SIZE
                + page_size.to_string().len().next_multiple_of(4);
            let semver_ext =
                Extension::HEADER_SIZE + semver.len().next_multiple_of(4);
            max_chunk_size -= target_page_size_ext + semver_ext;
        }

        // create block
        let chunk_size = max_chunk_size;
        let mut block = Block::new(
            *block_no,
            u32::MAX as usize, // Placeholder for total_blocks
            &page_data[offset..offset + chunk_size],
            page_addr + offset,
        );

        // Add family ID if provided
        if let Some(id) = family_id {
            block.set_family_id(id);
        }

        // Set the page_checksum for all blocks in the page
        block
            .set_checksum(&page_checksum)
            .map_err(FileError::BlockCorruption)?;

        // Add extensions to the first block of each page
        if offset == 0 {
            let page_size_str = page_size.to_string();
            block
                .add_extension(
                    ExtensionTag::TargetPageSize,
                    page_size_str.as_bytes(),
                )
                .map_err(FileError::BlockCorruption)?;
            let semver_str = semver.to_string();
            block
                .add_extension(
                    ExtensionTag::SemverString,
                    semver_str.as_bytes(),
                )
                .map_err(FileError::BlockCorruption)?;
        }

        offset += chunk_size;
        blocks.push(block);
        *block_no += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Read;
    use std::path::Path;

    #[test]
    fn test_is_uf2_file() {
        assert!(is_uf2_file(Path::new("test/example.uf2")));
    }

    #[test]
    fn test_is_uf2_buffer() {
        let mut f = File::open("test/example.uf2").unwrap();
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).unwrap();

        assert!(is_uf2_buffer(&buffer));
    }

    #[test]
    fn test_uf2_from_bytes() {
        let mut f = File::open("test/example.uf2").unwrap();
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).unwrap();

        let uf2_file = Uf2File::from_bytes(&buffer).unwrap();
        assert_eq!(uf2_file.blocks.len(), buffer.len() / BLOCK_SIZE);
    }

    #[test]
    fn test_uf2_from_file() {
        let uf2_file =
            Uf2File::from_file(Path::new("test/example.uf2")).unwrap();
        assert_eq!(uf2_file.blocks.len(), 1438);
    }

    #[test]
    fn test_list_family_ids() {
        let mut f = File::open("test/example.uf2").unwrap();
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).unwrap();
        let uf2_file = Uf2File::from_bytes(&buffer).unwrap();
        let family_ids = uf2_file.list_family_ids();
        assert_eq!(family_ids.len(), 0);
        //        assert_eq!(family_ids[0], 0xE48BFF56);
    }

    #[test]
    fn test_uf2_file_get_payload() {
        let mut f = File::open("test/example.uf2").unwrap();
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).unwrap();

        let uf2_file = Uf2File::from_bytes(&buffer).unwrap();
        let bytes = uf2_file.get_payload(None);
        assert_eq!(bytes.unwrap().len(), buffer.len() / 2);
    }

    #[test]
    fn test_uf2_file_verify() {
        let mut f = File::open("test/example.uf2").unwrap();
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer).unwrap();

        let uf2_file = Uf2File::from_bytes(&buffer).unwrap();
        assert!(uf2_file.verify().is_ok());
    }

    #[test]
    fn test_example_file() {
        use std::io::prelude::*;

        let mut f = std::fs::File::open("test/example.uf2").unwrap();
        let mut buffer = [0; 512];

        f.read(&mut buffer).unwrap();

        let block = Block::from_bytes(&buffer).unwrap();

        assert_eq!(block.target_addr, 0x2000);
        assert_eq!(block.data().len(), 256);
        assert_eq!(block.block_no, 0);
        assert_eq!(block.total_blocks, 1438);
        assert_eq!(block.has_family_id(), false);
    }

    #[test]
    fn test_add_binary_page_size_128() {
        let binary_data = vec![0xAA; 256]; // 256 bytes of data
        let target_addr = 0x2000;
        let page_size = 128;
        let semver = "1.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify the number of blocks
        assert_eq!(uf2_file.blocks.len(), 4);

        // Verify the blocks with extensions
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();
        assert_eq!(blocks_with_extensions.len(), 256 / 128);

        // Verify checksums
        for block in &uf2_file.blocks {
            assert!(block.has_checksum());
        }

        // Verify total_blocks
        for block in &uf2_file.blocks {
            assert_eq!(block.total_blocks, uf2_file.blocks.len() as u32);
        }
    }

    #[test]
    fn test_add_binary_empty_data() {
        let binary_data = vec![]; // Empty data
        let target_addr = 0x2000;
        let page_size = 128;
        let semver = "1.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify no blocks are created for empty data
        assert_eq!(uf2_file.blocks.len(), 0);
    }

    #[test]
    fn test_add_binary_large_data() {
        let binary_data = vec![0xFF; 2048]; // 2048 bytes of data
        let target_addr = 0x2000;
        let page_size = 256;
        let semver = "2.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify the number of blocks
        assert!(uf2_file.blocks.len() >= 16);

        // Verify the blocks with extensions
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();
        assert_eq!(blocks_with_extensions.len(), 2048 / 256); // 2048/256 = 8 flash pages
        for block in blocks_with_extensions {
            assert!(block.has_extension());
        }

        // Verify checksums
        for block in &uf2_file.blocks {
            assert!(block.has_checksum());
        }

        // Verify total_blocks
        for block in &uf2_file.blocks {
            assert_eq!(block.total_blocks, uf2_file.blocks.len() as u32);
        }
    }

    #[test]
    fn test_add_binary_page_size_256() {
        let binary_data = vec![0xBB; 512]; // 512 bytes of data
        let target_addr = 0x2000;
        let page_size = 256;
        let semver = "2.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify the number of blocks
        assert_eq!(uf2_file.blocks.len(), 4); // At least 4 blocks

        // Verify the blocks with extensions
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();
        assert_eq!(blocks_with_extensions.len(), 512 / 256);

        // Verify checksums
        for block in &uf2_file.blocks {
            assert!(block.has_checksum());
        }

        // Verify total_blocks
        for block in &uf2_file.blocks {
            assert_eq!(block.total_blocks, uf2_file.blocks.len() as u32);
        }
    }

    #[test]
    fn test_add_binary_page_size_512() {
        let binary_data = vec![0xCC; 1024]; // 1024 bytes of data
        let target_addr = 0x2000;
        let page_size = 512;
        let semver = "3.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify the number of blocks
        assert!(uf2_file.blocks.len() >= 4); // At least 4 blocks

        // Verify the blocks with extensions
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();
        assert_eq!(blocks_with_extensions.len(), 2); // Two flash pages
        for block in blocks_with_extensions {
            assert!(block.has_extension());
        }

        // Verify checksums
        for block in &uf2_file.blocks {
            assert!(block.has_checksum());
        }

        // Verify total_blocks
        for block in &uf2_file.blocks {
            assert_eq!(block.total_blocks, uf2_file.blocks.len() as u32);
        }
    }

    #[test]
    fn test_add_binary_page_size_1024() {
        let binary_data = vec![0xDD; 2048]; // 2048 bytes of data
        let target_addr = 0x2000;
        let page_size = 1024;
        let semver = "4.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Verify the number of blocks
        assert!(uf2_file.blocks.len() >= 6); // At least 6 blocks

        // Verify the blocks with extensions
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();
        assert_eq!(blocks_with_extensions.len(), 2048 / 1024); // Two flash pages
        for block in blocks_with_extensions {
            assert!(block.has_extension());
        }

        // Verify checksums
        for block in &uf2_file.blocks {
            assert!(block.has_checksum());
        }

        // Verify total_blocks
        for block in &uf2_file.blocks {
            assert_eq!(block.total_blocks, uf2_file.blocks.len() as u32);
        }
    }

    #[test]
    fn test_add_binary_address_alignment() {
        let binary_data = vec![0xEE; 3072]; // 3072 bytes of data (3 pages of 1024 bytes)
        let target_addr = 0x1000; // Start at address 0x1000
        let page_size = 1024;
        let semver = "5.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Find the first block of each page (blocks with extensions)
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();

        // Debug: print all page addresses
        for block in uf2_file.blocks.iter() {
            if block.has_extension() {
                println!(
                    "Block {} address: 0x{:x}, {} bytes, extension",
                    block.block_no,
                    block.target_addr,
                    block.data().len()
                );
            } else {
                println!(
                    "Block {} address: 0x{:x}, {} bytes",
                    block.block_no,
                    block.target_addr,
                    block.data().len()
                );
            }
        }

        // Verify we have 7 block
        assert!(uf2_file.blocks.iter().count() <= 9);
        // covering 3 pages
        assert_eq!(blocks_with_extensions.len(), 3);

        // Verify address alignment for each page
        // Page 0 should start at target_addr (0x1000)
        assert_eq!(blocks_with_extensions[0].target_addr, 0x1000);

        // Page 1 should be aligned to page_size boundary
        // 0x1000 + 1024 = 0x1400, which is already aligned
        assert_eq!(blocks_with_extensions[1].target_addr, 0x1400);

        // Page 2 should be aligned to page_size boundary
        // 0x1400 + 1024 = 0x1800, which is already aligned
        assert_eq!(blocks_with_extensions[2].target_addr, 0x1800);

        // Verify all page addresses are aligned to page_size boundaries
        for (i, block) in blocks_with_extensions.iter().enumerate() {
            assert_eq!(
                block.target_addr % page_size as u32,
                0,
                "Page {} address 0x{:x} not aligned to page size {}",
                i,
                block.target_addr,
                page_size
            );
        }
    }

    #[test]
    fn test_add_binary_misaligned_start() {
        let binary_data = vec![0xBE; 2048];
        let target_addr = 0x1234; // Start at misaligned address
        let page_size = 1024;
        let semver = "6.0.0";

        let mut uf2_file = Uf2File::new();
        uf2_file
            .add_binary(&binary_data, target_addr, None, page_size, semver)
            .unwrap();

        // Find the first block of each page (blocks with extensions)
        let blocks_with_extensions: Vec<_> = uf2_file
            .blocks
            .iter()
            .filter(|b| b.has_extension())
            .collect();

        // Debug: print all page addresses
        for block in uf2_file.blocks.iter() {
            if block.has_extension() {
                println!(
                    "Block {} address: 0x{:x}, {} bytes, extension",
                    block.block_no,
                    block.target_addr,
                    block.data().len()
                );
            } else {
                println!(
                    "Block {} address: 0x{:x}, {} bytes",
                    block.block_no,
                    block.target_addr,
                    block.data().len()
                );
            }
        }

        // Verify we have 7 block
        assert!(uf2_file.blocks.iter().count() <= 7);
        // covering 3 pages
        assert_eq!(blocks_with_extensions.len(), 3);

        // First page starts at the specified target_addr
        assert_eq!(blocks_with_extensions[0].target_addr, 0x1234);

        // Second page should be aligned to next page_size boundary
        // Aligned to 1024 boundary: ((0x1234 + 1023) / 1024) * 1024 = 0x1400
        assert_eq!(blocks_with_extensions[1].target_addr, 0x1400);
        assert_eq!(blocks_with_extensions[2].target_addr, 0x1800);

        let data = uf2_file.get_payload(None).unwrap();
        assert_eq!(data.len(), 2048);
    }
}
