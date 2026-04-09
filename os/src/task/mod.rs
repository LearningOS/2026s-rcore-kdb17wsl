//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
mod context;
mod id;
mod manager;
mod processor;
mod switch;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;

use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;
pub use context::TaskContext;
use lazy_static::*;
pub use manager::{fetch_task, TaskManager};
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::add_task;
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,
    Processor,
};
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    // There must be an application running.
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    // ---- release current PCB

    // push back to ready queue.
    add_task(task);
    // jump to scheduling cycle
    schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = take_current_task().unwrap();

    let pid = task.getpid();
    if pid == IDLE_PID {
        println!(
            "[kernel] Idle process exit with exit_code {} ...",
            exit_code
        );
        panic!("All applications completed!");
    }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
    inner.task_status = TaskStatus::Zombie;
    // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    // drop file descriptors
    inner.fd_table.clear();
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new({
        let inode = open_file("ch6b_initproc", OpenFlags::RDONLY).unwrap();
        let v = inode.read_all();
        TaskControlBlock::new(v.as_slice())
    });
}

///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
}


/// Map a new page for the current 'Running' task.
pub fn mmap(start: usize, len: usize, prot: usize) -> isize {
    if start % crate::config::PAGE_SIZE != 0 || (prot & !0x7) != 0 || (prot & 0x7) == 0 {
        return -1;
    }

    let end = match start.checked_add(len) {
        Some(v) => v,
        None => return -1,
    };

    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let memory_set = &mut task_inner.memory_set;

    let start_va = crate::mm::VirtAddr(start);
    let end_va = crate::mm::VirtAddr(end);
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();

    for vpn_idx in start_vpn.0..end_vpn.0 {
        let vpn = crate::mm::VirtPageNum(vpn_idx);
        if let Some(pte) = memory_set.translate(vpn) {
            if pte.is_valid() {
                return -1;
            }
        }
    }

    if start_vpn.0 == end_vpn.0 {
        return 0;
    }

    let mut perm = crate::mm::MapPermission::U;
    if (prot & 0x1) != 0 {
        perm |= crate::mm::MapPermission::R;
    }
    if (prot & 0x2) != 0 {
        perm |= crate::mm::MapPermission::W;
    }
    if (prot & 0x4) != 0 {
        perm |= crate::mm::MapPermission::X;
    }

    memory_set.insert_framed_area(start_va, end_va, perm);
    0
}

/// Unmap a page for the current 'Running' task.
pub fn munmap(start: usize, len: usize) -> isize {
    if start % crate::config::PAGE_SIZE != 0 {
        return -1;
    }

    let end = match start.checked_add(len) {
        Some(v) => v,
        None => return -1,
    };

    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let memory_set = &mut task_inner.memory_set;

    let start_va = crate::mm::VirtAddr(start);
    let end_va = crate::mm::VirtAddr(end);
    let start_vpn = start_va.floor();
    let end_vpn = end_va.ceil();

    for vpn_idx in start_vpn.0..end_vpn.0 {
        let vpn = crate::mm::VirtPageNum(vpn_idx);
        match memory_set.translate(vpn) {
            Some(pte) if pte.is_valid() => {}
            _ => return -1,
        }
    }

    if start_vpn.0 == end_vpn.0 {
        return 0;
    }

    memory_set.unmap_vpn_range(start_va, end_va);
    0
}