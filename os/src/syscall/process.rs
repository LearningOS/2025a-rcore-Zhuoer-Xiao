//! Process management syscalls
use alloc::sync::Arc;

use crate::{
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str, VirtAddr, MapPermission},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, translate_vpn_to_pte, current_memory_set,
    },
    timer::get_time_us,
    config::PAGE_SIZE,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    
    loop {
        // Check if there's any child process matching the given pid
        if !inner
            .children
            .iter()
            .any(|p| pid == -1 || pid as usize == p.getpid())
        {
            return -1;
            // ---- release current PCB
        }
        
        let pair = inner.children.iter().enumerate().find(|(_, p)| {
            // ++++ temporarily access child PCB exclusively
            p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
            // ++++ release child PCB
        });
        if let Some((idx, _)) = pair {
            let child = inner.children.remove(idx);
            // confirm that child will be deallocated after being removed from children list
            assert_eq!(Arc::strong_count(&child), 1);
            let found_pid = child.getpid();
            // ++++ temporarily access child PCB exclusively
            let exit_code = child.inner_exclusive_access().exit_code;
            // ++++ release child PCB
            *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
            return found_pid as isize;
        } else {
            // If no zombie child found, we need to yield and check again
            drop(inner); // Release the lock before yielding
            suspend_current_and_run_next();
            inner = task.inner_exclusive_access(); // Reacquire the lock
        }
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let sec= us / 1000000;
    let usec = us % 1000000;
    let tv=TimeVal{
        sec,
        usec
    };
    let ts_addr=ts as usize;

    // 将tv转换为字节数组
    let tv_bytes=unsafe{
        core::slice::from_raw_parts(
            &tv as * const TimeVal as *const u8, 
            core::mem::size_of::<TimeVal>()
        )
    };

    // 逐字节进行写入
    for (i,&byte) in tv_bytes.iter().enumerate(){
        let addr=ts_addr+i;
        let va=VirtAddr::from(addr);
        let vpn=va.floor();// 获取虚拟页号
        let offset=va.page_offset();

        if let Some(pte)=translate_vpn_to_pte(vpn){
            // 添加写权限检查
            if !pte.is_valid() || !pte.user_accessible() || !pte.writable() {
                return -1;
            }
            // 获取物理页号
            let ppn=pte.ppn();
            let dst=ppn.get_bytes_array();
            dst[offset]=byte;
        }else{
            return -1;
        }
    }
    0
}


/// 内存映射系统调用
/// 申请长度为len的物理内存，映射到start开始的虚拟地址处
/// 参数：
/// start: 内存映射的起始地址
/// len: 内存映射的长度
/// prot: 内存属性页标识
/// 可能的错误情况：
/// start地址不是页对齐地址
/// 物理内存不足
/// start,start+len存在已被映射页
/// prot参数错误：
/// prot & !0x7 != 0 (prot 其余位必须为0)
/// prot & 0x7 = 0 (这样的内存无意义)
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap start={:#x} len={} prot={:#x}",start,len,prot);
    // 检查地址是否按页对齐
    if start % PAGE_SIZE !=0{
        trace!("kernel: sys_mmap start is not aligned");
        return -1;
    }
    // 检查prot是否异常
    if prot & !0x7 != 0 || prot & 0x7 == 0{
        trace!("kernel: sys_mmap prot is invalid");
        return -1;
    }

    // 转换prot为MapPermission
    let mut map_perm=MapPermission::U;
    if prot & 0x1 != 0 {
        map_perm |= MapPermission::R;
    }
    if prot &0x2 !=0 {
        map_perm |= MapPermission::W;
    }

    if prot & 0x4 != 0 {
        map_perm |= MapPermission::X;
    }    
    
    current_memory_set(|memory_set|{
        if memory_set.mmap(VirtAddr::from(start),len,map_perm){
            0
        }else{
            -1
        }
    })
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    current_memory_set(|memory_set|{
        if memory_set.munmap(VirtAddr::from(start),len){
            0
        }else{
            -1
        }
    })
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn path={:?}",
        current_task().unwrap().pid.0,
        path
    );
    
    let token = current_user_token();
    let path_str = translated_str(token, path);
    
    if let Some(data) = get_app_data_by_name(path_str.as_str()) {
        let new_task = Arc::new(crate::task::TaskControlBlock::new(&data));
        let new_pid = new_task.pid.0;
        
        // 设置父子进程关系
        let current_task = current_task().unwrap();
        let mut parent_inner = current_task.inner_exclusive_access();
        let mut new_task_inner = new_task.inner_exclusive_access();
        new_task_inner.parent = Some(Arc::downgrade(&current_task));
        drop(new_task_inner);
        parent_inner.children.push(new_task.clone());
        drop(parent_inner);
        
        // 添加新任务到调度器
        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority prio={}",
        current_task().unwrap().pid.0,
        prio
    );
    
    // 检查优先级是否合法 (>= 2)
    if prio < 2 {
        return -1;
    }
    
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.priority = prio as usize;
    prio
}