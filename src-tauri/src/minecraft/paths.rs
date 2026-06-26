use std::{io, path::PathBuf};

pub const INSTANCE_GAME_DIR_NAME: &str = "game";

pub fn instance_game_dir(instance_id: &str) -> io::Result<PathBuf> {
    Ok(crate::instance::local_instance_dir(instance_id)?.join(INSTANCE_GAME_DIR_NAME))
}
