use crate::config::MountConfig;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Mounter {
    is_mounted: bool,
    device: String,
    is_used: bool,
    mountpoint: String,
    is_luks: bool,
    pw: String,
}

impl Mounter {
    pub fn new(config: &Option<MountConfig>) -> Mounter {
        let mut is_used = false;
        let mut is_luks = false;
        let mut device: String = String::from("");
        let mut mountpoint: String = String::from("");
        let mut pw: String = String::from("");

        if let Some(config) = &config {
            if let Some(uuid) = &config.uuid {
                device = String::from("/dev/disk/by-uuid/") + &uuid;
                if let Some(mp) = &config.mountpoint {
                    mountpoint = mp.to_string();
                } else {
                    mountpoint = String::from("/mnt/backapp/") + &uuid;
                }
                is_used = true;
                if let Some(password) = &config.password {
                    is_luks = true;
                    pw = password.to_string();
                }
            }
        }
        Mounter {
            is_mounted: false,
            device,
            is_used,
            mountpoint,
            is_luks,
            pw,
        }
    }
    pub fn mount(&self) -> Result<(), &str> {
        if !self.is_used {
            return Ok(());
        }
        if !self.is_mounted {
            log::info!("Mounting {} at {}", self.device, self.mountpoint);
            if !std::path::Path::new(&self.device).exists() {
                log::error!("Device not existing: {}", self.device);
                return Err("device not found");
            }
            if !std::path::Path::new(&self.mountpoint).exists() {
                match std::fs::create_dir_all(&self.mountpoint) {
                    Ok(_) => log::warn!("Create mountpoint: {}", self.mountpoint),
                    Err(_) => {
                        log::error!("Failed to create mountpoint: {}", self.mountpoint);
                        return Err("Mountpoint cannot be created!");
                    }
                }
            }
            // TODO: ok, device and mp exist, check luks and do the mount
        } else {
            log::warn!(
                "Device {} already mounted (duplicate mount request)!",
                self.device
            );
        }
        Ok(())
    }
    pub fn umount(&self) -> Result<(), &str> {
        log::info!("Unmounting {} from {}", self.device, self.mountpoint);
        if self.is_mounted {
            // TODO: cryptosetup - luksClose optional  <03-01-21, Heiko Riemer> //
            // Command::new("umount")
            //     .arg(&self.mountpoint)
            //     .output()
            //     .expect("unable to umount device");
        }
        Ok(())
    }
}

impl Drop for Mounter {
    fn drop(&mut self) {
        if self.is_mounted {
            match self.umount() {
                Ok(_) => log::info!("Umount {}", self.device),
                Err(_) => log::warn!("Umount failed on {}", self.device),
            }
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
