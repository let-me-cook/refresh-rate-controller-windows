use std::collections::HashSet;
use std::mem;
use std::ptr;

use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::DWORD;
use winapi::um::cfgmgr32::{CM_DRP_DEVICEDESC, CM_DRP_FRIENDLYNAME};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::setupapi::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceRegistryPropertyW, DIGCF_PRESENT, DIGCF_PROFILE, HDEVINFO, SP_DEVINFO_DATA,
};
use winapi::um::wingdi::{
    DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_PRIMARY_DEVICE, DM_DISPLAYFREQUENCY,
};
use winapi::um::winnt::WCHAR;
use winapi::um::winuser::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, DISP_CHANGE_RESTART,
    DISP_CHANGE_SUCCESSFUL, ENUM_CURRENT_SETTINGS,
};

pub fn to_wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get_available_refresh_rates(device_name_wide: &[u16]) -> Vec<DWORD> {
    let mut refresh_rates = HashSet::new();
    let mut dev_mode: DEVMODEW = unsafe { mem::zeroed() };
    dev_mode.dmSize = mem::size_of::<DEVMODEW>() as u16;

    let mut mode_num = 0;
    loop {
        let result =
            unsafe { EnumDisplaySettingsW(device_name_wide.as_ptr(), mode_num, &mut dev_mode) };

        if result == 0 {
            break;
        }

        if dev_mode.dmDisplayFrequency > 1 {
            refresh_rates.insert(dev_mode.dmDisplayFrequency);
        }
        mode_num += 1;
    }

    let mut sorted_rates: Vec<DWORD> = refresh_rates.into_iter().collect();
    sorted_rates.sort_unstable();
    sorted_rates
}

#[derive(Debug, Clone)]
pub struct DisplayDevice {
    pub device_name: String,
    pub display_name: String,
}

pub fn get_all_display_devices() -> Vec<DisplayDevice> {
    let mut devices = Vec::new();
    let mut adapter_device: DISPLAY_DEVICEW = unsafe { mem::zeroed() };
    adapter_device.cb = mem::size_of::<DISPLAY_DEVICEW>() as DWORD;

    // GUID for monitor devices (GUID_DEVCLASS_MONITOR)
    // {4d36e96e-e325-11ce-bfc1-08002be10318}
    let guid_devclass_monitor: GUID = GUID {
        Data1: 0x4d36e96e,
        Data2: 0xe325,
        Data3: 0x11ce,
        Data4: [0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1, 0x03, 0x18],
    };

    let hdevinfo: HDEVINFO = unsafe {
        SetupDiGetClassDevsW(
            &guid_devclass_monitor,
            ptr::null_mut(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_PROFILE, // Only devices that are currently present, and include profile-specific devices
        )
    };

    if hdevinfo == INVALID_HANDLE_VALUE {
        eprintln!("Error: SetupDiGetClassDevsW failed. Last Error: {}", unsafe { GetLastError() });
        return devices;
    }

    // Enumerate display adapters
    for adapter_idx in 0.. {
        let result = unsafe { EnumDisplayDevicesW(ptr::null_mut(), adapter_idx, &mut adapter_device, 0) };
        if result == 0 {
            break; // No more adapters
        }

        // Check if the adapter is active and attached to the desktop
        if adapter_device.StateFlags & winapi::um::wingdi::DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0 {
            let adapter_device_name = String::from_utf16_lossy(&adapter_device.DeviceName)
                .trim_end_matches('\0')
                .to_string();

            let mut monitor_device: DISPLAY_DEVICEW = unsafe { mem::zeroed() };
            monitor_device.cb = mem::size_of::<DISPLAY_DEVICEW>() as DWORD;

            // Enumerate monitors for this adapter
            for monitor_idx in 0.. {
                let result = unsafe {
                    EnumDisplayDevicesW(
                        to_wide_string(&adapter_device_name).as_ptr(),
                        monitor_idx,
                        &mut monitor_device,
                        0,
                    )
                };
                if result == 0 {
                    break; // No more monitors for this adapter
                }

                // Check if the monitor is active
                if monitor_device.StateFlags & winapi::um::wingdi::DISPLAY_DEVICE_ACTIVE != 0 {
                    let mut monitor_display_name = String::from_utf16_lossy(&monitor_device.DeviceString)
                        .trim_end_matches('\0')
                        .to_string();

                    println!("DEBUG: Original monitor_device.DeviceString: {}", monitor_display_name);

                    // Try to get a more accurate name using SetupDiGetDeviceRegistryPropertyW
                    let mut device_info_data: SP_DEVINFO_DATA = unsafe { mem::zeroed() };
                    device_info_data.cbSize = mem::size_of::<SP_DEVINFO_DATA>() as DWORD;

                    // Find the corresponding device info for the monitor
                    for i in 0.. {
                        let enum_dev_result = unsafe {
                            SetupDiEnumDeviceInfo(hdevinfo, i, &mut device_info_data)
                        };
                        if enum_dev_result == 0 {
                            break;
                        }

                        let mut buffer: Vec<u16> = vec![0; 256]; // Adjust size as needed
                        let mut required_size: DWORD = 0;
                        let mut monitor_display_name_candidate = String::new();

                        // Try to get CM_DRP_FRIENDLYNAME first
                        let get_friendly_name_result = unsafe {
                            SetupDiGetDeviceRegistryPropertyW(
                                hdevinfo,
                                &mut device_info_data,
                                CM_DRP_FRIENDLYNAME, // <<< FIRST ATTEMPT: Friendly Name
                                ptr::null_mut(),
                                buffer.as_mut_ptr() as *mut u8,
                                (buffer.len() * mem::size_of::<WCHAR>()) as DWORD,
                                &mut required_size,
                            )
                        };

                        if get_friendly_name_result != 0 {
                            let friendly_name = String::from_utf16_lossy(&buffer[..(required_size / mem::size_of::<WCHAR>() as DWORD) as usize])
                                .trim_end_matches('\0')
                                .to_string();
                            if !friendly_name.is_empty() && friendly_name != "Generic PnP Monitor" {
                                monitor_display_name_candidate = friendly_name;
                                println!("DEBUG: Retrieved display name from CM_DRP_FRIENDLYNAME: {}", monitor_display_name_candidate);
                            }
                        }

                        // Fallback to CM_DRP_DEVICEDESC if friendly name was not ideal
                        if monitor_display_name_candidate.is_empty() || monitor_display_name_candidate == "Generic PnP Monitor" {
                            let get_desc_result = unsafe {
                                SetupDiGetDeviceRegistryPropertyW(
                                    hdevinfo,
                                    &mut device_info_data,
                                    CM_DRP_DEVICEDESC, // <<< FALLBACK: Device Description
                                    ptr::null_mut(),
                                    buffer.as_mut_ptr() as *mut u8,
                                    (buffer.len() * mem::size_of::<WCHAR>()) as DWORD,
                                    &mut required_size,
                                )
                            };

                            if get_desc_result != 0 {
                                let device_description = String::from_utf16_lossy(&buffer[..(required_size / mem::size_of::<WCHAR>() as DWORD) as usize])
                                    .trim_end_matches('\0')
                                    .to_string();
                                if !device_description.is_empty() && device_description != "Generic PnP Monitor" {
                                    monitor_display_name_candidate = device_description;
                                    println!("DEBUG: Retrieved display name from CM_DRP_DEVICEDESC (fallback): {}", monitor_display_name_candidate);
                                }
                            }
                        }

                        // Ensure monitor_display_name gets the best candidate
                        monitor_display_name = if !monitor_display_name_candidate.is_empty() {
                            monitor_display_name_candidate
                        } else {
                            // If all else fails, use the original DeviceString (which might be "Generic PnP Monitor")
                            String::from_utf16_lossy(&monitor_device.DeviceString)
                                .trim_end_matches('\0')
                                .to_string()
                        };
                        break; // Found a name, no need to check other device infos
                    }

                    // Use the adapter's device name for setting refresh rates, but the monitor's display name for UI
                    devices.push(DisplayDevice {
                        device_name: adapter_device_name.clone(),
                        display_name: monitor_display_name,
                    });
                }
            }
        }
    }

    unsafe { SetupDiDestroyDeviceInfoList(hdevinfo) };
    devices
}

pub fn get_primary_display_device_name() -> Option<String> {
    let mut display_device: DISPLAY_DEVICEW = unsafe { mem::zeroed() };
    display_device.cb = mem::size_of::<DISPLAY_DEVICEW>() as DWORD;

    for i in 0.. {
        let result = unsafe { EnumDisplayDevicesW(ptr::null_mut(), i, &mut display_device, 0) };
        if result == 0 {
            break;
        }

        let device_name_str = String::from_utf16_lossy(&display_device.DeviceName)
            .trim_end_matches('\0')
            .to_string();

        if display_device.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0 {
            return Some(device_name_str);
        }
    }
    None
}

pub fn set_display_refresh_rate(device_name: &str, refresh_rate: DWORD) -> bool {
    let device_name_wide = to_wide_string(device_name);
    let mut dev_mode: DEVMODEW = unsafe { mem::zeroed() };
    dev_mode.dmSize = mem::size_of::<DEVMODEW>() as u16;

    println!(
        "DEBUG: Enumerating settings for primary display: {}",
        device_name
    );

    let enum_settings_result = unsafe {
        EnumDisplaySettingsW(
            device_name_wide.as_ptr(),
            ENUM_CURRENT_SETTINGS,
            &mut dev_mode,
        )
    };

    if enum_settings_result == 0 {
        eprintln!(
            "Error: Could not enumerate current display settings for {}. Last Error: {}",
            device_name,
            unsafe { GetLastError() }
        );
        return false;
    }

    // Only change refresh rate if it's different to avoid unnecessary mode changes
    if dev_mode.dmDisplayFrequency == refresh_rate {
        println!(
            "Refresh rate for {} is already {} Hz. No change needed.",
            device_name, refresh_rate
        );
        return true;
    }

    dev_mode.dmDisplayFrequency = refresh_rate;
    dev_mode.dmFields |= DM_DISPLAYFREQUENCY;

    let change_result = unsafe {
        ChangeDisplaySettingsExW(
            device_name_wide.as_ptr(),
            &mut dev_mode,
            ptr::null_mut(),
            0, // 0 for immediate application
            ptr::null_mut(),
        )
    };

    match change_result {
        DISP_CHANGE_SUCCESSFUL => {
            println!(
                "Successfully changed refresh rate for {} to {} Hz.",
                device_name, refresh_rate
            );
            true
        }
        DISP_CHANGE_RESTART => {
            println!("Refresh rate for {} changed, but a restart is required for changes to take full effect.", device_name);
            true
        }
        _ => {
            eprintln!(
                "Failed to change refresh rate for {} to {} Hz. Error code: {}. Last Error: {}",
                device_name,
                refresh_rate,
                change_result,
                unsafe { GetLastError() }
            );
            false
        }
    }
}
