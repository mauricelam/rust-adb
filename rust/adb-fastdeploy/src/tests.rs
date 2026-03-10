#[cfg(test)]
mod tests {
    use crate::apk_archive::ApkArchive;
    use crate::deploy_patch_generator::DeployPatchGenerator;
    use crate::patch_utils::{PatchUtils, K_SIGNATURE};
    use crate::proto::ApkMetaData;
    use prost::Message;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn get_test_file(name: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("../../original/fastdeploy/testdata");
        path.push(name);
        path
    }

    #[test]
    fn test_apk_archive_sizes() {
        let path = get_test_file("sample.apk");
        let mut archive = ApkArchive::open(&path).unwrap();

        let cd_loc = archive.get_cd_location().unwrap();
        assert!(cd_loc.valid);
        assert_eq!(cd_loc.offset, 2044145);
        assert_eq!(cd_loc.size, 49390);

        let sig_loc = archive.get_signature_location(cd_loc.offset).unwrap();
        assert!(sig_loc.valid);
        assert_eq!(sig_loc.offset, 2040049);
        assert_eq!(sig_loc.size, 4088);
    }

    #[test]
    fn test_apk_archive_dump() {
        let path = get_test_file("sample.apk");
        let mut archive = ApkArchive::open(&path).unwrap();

        let dump = archive.extract_metadata().unwrap();
        assert_eq!(dump.cd.len(), 49390);
        assert_eq!(dump.signature.len(), 4088);
    }

    #[test]
    fn test_swap_long_writes() {
        let mut output = Vec::new();
        PatchUtils::write_long(0x0011223344556677, &mut output).unwrap();
        let expected = [0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00];
        assert_eq!(output, expected);
    }

    #[test]
    fn test_pipe_writes_amount_to_output() {
        let expected = b"Some Data";
        let mut input = &expected[..];
        let mut output = Vec::new();
        PatchUtils::pipe(&mut input, &mut output, expected.len() as u64).unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn test_signature_const_matches() {
        let mut output = Vec::new();
        PatchUtils::write_signature(&mut output).unwrap();
        assert_eq!(output, K_SIGNATURE);
    }

    #[test]
    fn test_gather_metadata() {
        let apk_file = get_test_file("rotating_cube-release.apk");
        let actual = PatchUtils::get_host_apk_metadata(&apk_file).unwrap();

        let expected_metadata_bytes = fs::read(get_test_file("rotating_cube-metadata-release.data")).unwrap();
        let expected = ApkMetaData::decode(&expected_metadata_bytes[..]).unwrap();

        // Actual path might vary, so we don't compare it directly if it's different in the test data.
        // But here we want to ensure entries are correct.
        assert_eq!(actual.entries.len(), expected.entries.len());
        for (a, e) in actual.entries.iter().zip(expected.entries.iter()) {
            assert_eq!(a.md5, e.md5);
            assert_eq!(a.data_offset, e.data_offset);
            // In the original test, they might clear data_size or it might differ.
            // Let's see if they match.
            assert_eq!(a.data_size, e.data_size);
        }
    }

    #[test]
    fn test_identical_file_entries() {
        let apk_path = get_test_file("rotating_cube-release.apk");
        let metadata_a = PatchUtils::get_host_apk_metadata(&apk_path).unwrap();
        let generator = DeployPatchGenerator::new(false);
        let mut entries = Vec::new();
        generator.build_identical_entries(&mut entries, &metadata_a, &metadata_a);
        assert_eq!(entries.len(), metadata_a.entries.len());
    }

    #[test]
    fn test_no_device_metadata() {
        let apk_path = get_test_file("rotating_cube-release.apk");
        let apk_size = fs::metadata(&apk_path).unwrap().len();

        let output_file = NamedTempFile::new().unwrap();
        let generator = DeployPatchGenerator::new(true);
        generator.create_patch(&apk_path, ApkMetaData::default(), &output_file).unwrap();

        let patch_size = output_file.as_file().metadata().unwrap().len();
        assert!(patch_size > apk_size);
    }

    #[test]
    fn test_zero_size_patch() {
        let apk_path = get_test_file("rotating_cube-release.apk");
        let mut archive = ApkArchive::open(&apk_path).unwrap();
        let dump = archive.extract_metadata().unwrap();
        assert!(!dump.cd.is_empty());

        let metadata = PatchUtils::get_device_apk_metadata(&dump);

        let output_file = NamedTempFile::new().unwrap();
        let generator = DeployPatchGenerator::new(true);
        generator.create_patch(&apk_path, metadata, &output_file).unwrap();

        let patch_size = output_file.as_file().metadata().unwrap().len();
        // The original test expects <= 512.
        assert!(patch_size <= 512);
    }
}
