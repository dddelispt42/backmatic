use crate::config::DestMountConfig;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Mounter {
    is_mounted: bool,
    uuid: String,
    device: String,
    is_used: bool,
    mountpoint: String,
    is_luks: bool,
    pw: String,
}

impl Mounter {
    pub fn new(config: &Option<DestMountConfig>) -> Mounter {
        let mut is_used = false;
        let mut is_luks = false;
        let mut uuid: String = String::from("");
        let mut device: String = String::from("");
        let mut mountpoint: String = String::from("");
        let mut pw: String = String::from("");

        if let Some(config) = &config {
            if let Some(id) = &config.uuid {
                uuid = id.to_string();
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
            uuid,
            device,
            is_used,
            mountpoint,
            is_luks,
            pw,
        }
    }
    pub fn mount(&mut self) -> Result<(), &str> {
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
            let mut mp = String::from(&self.mountpoint);
            if self.is_luks {
                let mut cmd = Command::new(format!("echo {} | cryptsetup luksOpen {} {}", self.pw, self.device, self.uuid));
                log::error!("Cryptsetup cmd: {:?}", cmd);
                let output = cmd.output().expect("unable to map crypto device");
                if !output.status.success() {
                    return Err("cryptsetup luksOpen failed");
                }
                mp = String::from("/dev/mapper/");
                mp.push_str(&self.uuid)
            }
            let mut cmd = Command::new("mount");
                cmd.arg(&self.device)
                .arg(mp);
            log::debug!("Mount cmd: {:?}", cmd);
            let output = cmd.output().expect("unable to mount device");
            if !output.status.success() {
                return Err("mounting disk failed");
            }
            self.is_mounted = true;
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
            let mut cmd = Command::new("umount");
            cmd.arg(&self.mountpoint);
            let output = cmd.output().expect("unable to umount device");
            if !output.status.success() {
                return Err("unmounting disk failed");
            }
            if self.is_luks {
                let mut cmd = Command::new(format!("cryptsetup luksClose {}", self.uuid));
                log::error!("Cryptsetup cmd: {:?}", cmd);
                let output = cmd.output().expect("unable to unmap crypto device");
                if !output.status.success() {
                    return Err("cryptsetup unmapping device failed");
                }
            }
        }
        Ok(())
    }
}

impl Drop for Mounter {
    fn drop(&mut self) {
        if self.is_mounted {
            match self.umount() {
                Ok(_) => log::debug!("Umount {} - Mounter destructor", self.device),
                Err(_) => log::warn!("Umount failed on {}", self.device),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    

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
