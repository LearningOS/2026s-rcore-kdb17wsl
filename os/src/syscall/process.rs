//! Process management syscalls
use crate::{
    mm::{self, translated_byte_buffer},
    task::{
        change_program_brk, current_user_token, exit_current_and_run_next, get_current_syscall_count, mmap, munmap, suspend_current_and_run_next
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let token = current_user_token();
    let timeval = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let len = core::mem::size_of::<TimeVal>();
    let buffer = &timeval as *const TimeVal as *const u8;
    let slice = unsafe { core::slice::from_raw_parts(buffer, len) };
    let user_buffer = _ts as usize;

    let translated = translated_byte_buffer(token, user_buffer as *const u8, len);

    let mut offset = 0;
    for buf in translated {
        let chunk_len = buf.len();
        buf.copy_from_slice(&slice[offset..offset + chunk_len]);
        offset += chunk_len;
    }

    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    let page_table = mm::PageTable::from_token(token);
    let va = mm::VirtAddr::from(_id);
    match _trace_request {
        0 => {
            if let Some(pte) = page_table.translate(va.floor()) {
                if pte
                    .flags()
                    .contains(mm::PTEFlags::V | mm::PTEFlags::U | mm::PTEFlags::R)
                {
                    let ppn = pte.ppn();
                    let pa = mm::PhysAddr::from(ppn).0 + va.page_offset();
                    return unsafe { *(pa as *const u8) } as isize;
                }
            }
            return -1;
        }
        1 => {
            if let Some(pte) = page_table.translate(va.floor()) {
                let flags = pte.flags();
                if flags.contains(mm::PTEFlags::V | mm::PTEFlags::U | mm::PTEFlags::W) {
                    let pa = mm::PhysAddr::from(pte.ppn()).0 + va.page_offset();
                    unsafe {
                        *(pa as *mut u8) = _data as u8;
                    }
                    return 0;
                }
            }
            return -1;
        }
        2 => {
            get_current_syscall_count(_id) as isize
        },
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    return mmap(_start, _len, _port);
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    return munmap(_start, _len);
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
