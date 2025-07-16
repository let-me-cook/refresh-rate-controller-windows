use std::mem;
use std::ptr;

use winapi::shared::minwindef::{DWORD, LOWORD, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, POINT};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use winapi::um::winuser::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DispatchMessageW,  GetCursorPos,
    GetMessageW, LoadIconW, PostQuitMessage, RegisterClassExW, SetForegroundWindow, ShowWindow,
    TrackPopupMenuEx, TranslateMessage, UpdateWindow, CW_USEDEFAULT,  IDC_ARROW, IDI_APPLICATION, MSG, SW_HIDE,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, TPM_TOPALIGN, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_RBUTTONUP, WM_USER, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};
use refresh_rate_windows_rs::{
    get_available_refresh_rates, get_all_display_devices, set_display_refresh_rate,
    to_wide_string, DisplayDevice,
};
use std::collections::HashMap;
use std::sync::LazyLock;

const WM_APP_NOTIFYICON: UINT = WM_USER + 1;

const MENU_REFRESH_RATE_BASE_ID: UINT = 2000; // Offset to avoid clashes
const MENU_EXIT_ID: UINT = 9999;

extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    static mut ALL_DISPLAY_DEVICES: Option<Vec<DisplayDevice>> = None;
    static DEVICE_REFRESH_RATES: LazyLock<std::sync::Mutex<HashMap<String, Vec<DWORD>>>> =
        LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

    match msg {
        WM_CREATE => {
            let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
            nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as DWORD;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_APP_NOTIFYICON;

            nid.hIcon = unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) };

            let tip_text = to_wide_string("Refresh Rate Tray");
            unsafe {
                ptr::copy_nonoverlapping(
                    tip_text.as_ptr(),
                    nid.szTip.as_mut_ptr(),
                    tip_text.len().min(nid.szTip.len() - 1),
                );
            }

            unsafe {
                Shell_NotifyIconW(NIM_ADD, &mut nid);
            }
            0
        }
        WM_APP_NOTIFYICON => {
            match LOWORD(lparam as DWORD) as UINT {
                WM_RBUTTONUP => {
                    // On Right-click
                    let mut pt: POINT = unsafe { mem::zeroed() };
                    unsafe { GetCursorPos(&mut pt) };

                    let hmenu = unsafe { CreatePopupMenu() };
                    if hmenu.is_null() {
                        eprintln!("Failed to create popup menu. Last Error: {}", unsafe {
                            GetLastError()
                        });
                        return 0;
                    }

                    // Dynamically get all display devices and their available refresh rates
                    unsafe {
                        ALL_DISPLAY_DEVICES = Some(get_all_display_devices());
                        let mut device_refresh_rates_guard = DEVICE_REFRESH_RATES.lock().unwrap();
                        device_refresh_rates_guard.clear(); // Clear previous rates

                        if let Some(devices) = ALL_DISPLAY_DEVICES.as_ref() {
                            for device in devices {
                                let device_name_wide = to_wide_string(&device.device_name);
                                let rates = get_available_refresh_rates(&device_name_wide);
                                device_refresh_rates_guard.insert(device.device_name.clone(), rates);
                            }
                        }
                    }

                    // Add monitor submenus
                    unsafe {
                        if let Some(devices) = ALL_DISPLAY_DEVICES.as_ref() {
                            if devices.is_empty() {
                                let no_monitors_text = to_wide_string("No monitors found");
                                AppendMenuW(hmenu, 0, 0, no_monitors_text.as_ptr());
                            } else {
                                for (i, device) in devices.iter().enumerate() {
                                    let submenu = CreatePopupMenu();
                                    if submenu.is_null() {
                                        eprintln!("Failed to create submenu for {}. Last Error: {}", device.display_name, GetLastError());
                                        continue;
                                    }

                                    let monitor_display_name = &device.display_name;
                                    let monitor_menu_text = to_wide_string(monitor_display_name);

                                    // Add refresh rates to submenu
                                    let device_refresh_rates_guard = DEVICE_REFRESH_RATES.lock().unwrap();
                                    if let Some(rates) = device_refresh_rates_guard.get(&device.device_name) {
                                        if rates.is_empty() {
                                            let no_rates_text = to_wide_string("No rates");
                                            AppendMenuW(submenu, 0, 0, no_rates_text.as_ptr());
                                        } else {
                                            for (j, &rate) in rates.iter().enumerate() {
                                                let rate_menu_text = to_wide_string(&format!("{} Hz", rate));
                                                AppendMenuW(
                                                    submenu,
                                                    0,
                                                    (MENU_REFRESH_RATE_BASE_ID + (i * 100) as UINT + j as UINT) as usize, // Unique ID for each rate
                                                    rate_menu_text.as_ptr(),
                                                );
                                            }
                                        }
                                    } else {
                                        let no_rates_text = to_wide_string("No rates");
                                        AppendMenuW(submenu, 0, 0, no_rates_text.as_ptr());
                                    }

                                    AppendMenuW(
                                        hmenu,
                                        0x00000010, // MF_POPUP
                                        submenu as usize,
                                        monitor_menu_text.as_ptr(),
                                    );
                                }
                            }
                        }
                    }

                    // Add a separator and Exit
                    let separator_text = to_wide_string("-");
                    unsafe {
                        AppendMenuW(hmenu, 0x00000800, 0, separator_text.as_ptr());
                        // MF_SEPARATOR
                    }
                    let exit_text = to_wide_string("Exit");
                    unsafe {
                        AppendMenuW(hmenu, 0, MENU_EXIT_ID as usize, exit_text.as_ptr());
                    }

                    // Set the foreground window to our hidden window before showing the menu.
                    // This is crucial for the menu to disappear when clicking elsewhere.
                    unsafe { SetForegroundWindow(hwnd) };

                    unsafe {
                        TrackPopupMenuEx(
                            hmenu,
                            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
                            pt.x,
                            pt.y,
                            hwnd,
                            ptr::null_mut(),
                        );
                    }

                    // Destroy the menu after use.
                    unsafe { DestroyMenu(hmenu) };
                    0
                }
                _ => 0,
            }
        }
        WM_COMMAND => {
            // Handle menu item selections
            let menu_id = LOWORD(wparam as DWORD) as UINT;
            unsafe {
                if menu_id >= MENU_REFRESH_RATE_BASE_ID {
                    // This is a refresh rate selection
                    let monitor_index = ((menu_id - MENU_REFRESH_RATE_BASE_ID) / 100) as usize;
                    let rate_index = ((menu_id - MENU_REFRESH_RATE_BASE_ID) % 100) as usize;

                    if let Some(devices) = ALL_DISPLAY_DEVICES.as_ref() {
                        if monitor_index < devices.len() {
                            let device = &devices[monitor_index];
                            let device_refresh_rates_guard = DEVICE_REFRESH_RATES.lock().unwrap();
                            if let Some(rates) = device_refresh_rates_guard.get(&device.device_name) {
                                if rate_index < rates.len() {
                                    let selected_rate = rates[rate_index];
                                    set_display_refresh_rate(&device.device_name, selected_rate);
                                } else {
                                    eprintln!("Error: Refresh rate index out of bounds.");
                                }
                            } else {
                                eprintln!("Error: Refresh rates not found for device: {}", device.device_name);
                            }
                        } else {
                            eprintln!("Error: Monitor index out of bounds.");
                        }
                    } else {
                        eprintln!("Error: Display devices not available.");
                    }
                } else if menu_id == MENU_EXIT_ID {
                    // Exit
                    PostQuitMessage(0);
                }
            }
            0
        }
        WM_DESTROY => {
            // Remove the tray icon when the window is destroyed
            let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
            nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as DWORD;
            nid.hWnd = hwnd;
            nid.uID = 1;
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &mut nid);
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => {
            // Default message processing.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn main() {
    // Get the instance handle for the application.
    let hinstance = unsafe { GetModuleHandleW(ptr::null_mut()) };

    // Define the window class name.
    let class_name = to_wide_string("RefreshRateTrayClass");

    // Register the window class
    let mut wc: WNDCLASSEXW = unsafe { mem::zeroed() };
    wc.cbSize = mem::size_of::<WNDCLASSEXW>() as UINT;
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hIcon = unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) };
    wc.hCursor = unsafe { LoadIconW(ptr::null_mut(), IDC_ARROW) as *mut _ };
    wc.lpszClassName = class_name.as_ptr();

    if unsafe { RegisterClassExW(&mut wc) } == 0 {
        eprintln!("Failed to register window class, error: {}", unsafe {
            GetLastError()
        });
        return;
    }

    // Create a hidden window. This window will receive messages for the tray icon
    let window_name = to_wide_string("Refresh Rate Tray Hidden Window");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        )
    };

    if hwnd.is_null() {
        eprintln!("Failed to create window, error: {}", unsafe {
            GetLastError()
        });
        return;
    }

    // Hide the window immediately
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    unsafe { UpdateWindow(hwnd) };

    // Message loop
    let mut msg: MSG = unsafe { mem::zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if ret == 0 {
            // WM_QUIT received, exit loop
            break;
        } else if ret == -1 {
            eprintln!("Error in message loop, error: {}", unsafe {
                GetLastError()
            });
            break;
        } else {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
