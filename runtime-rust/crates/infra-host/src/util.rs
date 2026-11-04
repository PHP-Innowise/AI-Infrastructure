pub fn private_directory(path: &std::path::Path) -> crate::Result<()> {
    infra_core::security::secure_private_dir(path).map_err(Into::into)
}
pub fn private_file(path: &std::path::Path) -> crate::Result<()> {
    infra_core::security::secure_private_file(path).map_err(Into::into)
}
