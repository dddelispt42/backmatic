use std::process::Command;

#[derive(Clone, Debug)]
pub struct Mounter {
    uuid: String,
    mount: String,
    pw: Option<String>,
    is_mounted: bool,
    _device: String,
}

impl Mounter {
    pub fn new(uuid: &str, pw: Option<String>) -> Mounter {
        let mut mount_point = String::from("/mnt/backapp");
        mount_point.push_str(uuid);
        let mut device = String::from("/dev/disk/by-uuid/");
        device.push_str(uuid);
        Mounter {
            uuid: String::from(uuid),
            mount: mount_point,
            pw,
            is_mounted: false,
            _device: device,
        }
    }
    pub fn mount(&self) -> Result<&str, &str> {
        log::info!("Mounting {} at {}", self.uuid, self.mount);
        if !std::path::Path::new(&self.mount).exists() {
            return Err("disk not available");
        }
        // TODO:cryptosetup - luksOpen optional  <03-01-21, Heiko Riemer> //
        Command::new("mkdir")
            .arg("-p")
            .arg(&self.mount)
            .output()
            .expect("unable to create mount point");
        Command::new("mount")
            .arg(&self._device)
            .arg(&self.mount)
            .output()
            .expect("unable to mount device");
        Ok(&self.mount)
    }
    pub fn umount(&self) -> Result<&str, &str> {
        log::info!("Unmounting {} from {}", self.uuid, self.mount);
        if self.is_mounted {
            Command::new("umount")
                .arg(&self.mount)
                .output()
                .expect("unable to umount device");
        }
        // TODO:cryptosetup - luksClose optional  <03-01-21, Heiko Riemer> //
        Ok(&self.mount)
    }
}

impl Drop for Mounter {
    fn drop(&mut self) {
        if self.is_mounted {
            self.umount().expect("unmounting failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mount_plain() {
        assert!(false)
    }
    #[test]
    fn test_umount_plain() {
        assert!(false)
    }
    #[test]
    fn test_mount_encrypted() {
        assert!(false)
    }
    #[test]
    fn test_umount_encrypted() {
        assert!(false)
    }
    #[test]
    fn test_mount_unknown_device() {
        assert!(false)
    }
    #[test]
    fn test_mount_encrypted_wrong_pw() {
        assert!(false)
    }
}
