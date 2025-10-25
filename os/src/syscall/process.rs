//! Process management syscalls
use crate::{config::PAGE_SIZE, mm::{MapPermission, VirtAddr}, task::{change_program_brk, current_momory_set, exit_current_and_run_next, get_syscall_count, suspend_current_and_run_next}, timer::get_time_us};
use crate::task::translate_vpn_to_pte;
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
            if !pte.is_valid(){
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

/// 跟踪系统调用
/// 参数：
/// trace_request: 操作类型
/// 0 读取id地址的一个字节
/// 1 写入data到id地址处
/// 2 查询syscall为id的系统调用次数
/// 返回值
/// trace_request=0:成功返回读取的值，失败返回-1
/// trace_request=1:成功返回0，失败返回-1
/// trace_request=2:成功返回系统调用次数，失败返回-1
/// 错误情况（trace_request=0/1）:
/// id对应的地址不存在
/// 地址用户不可见（U）
/// 页面不可读（R）、写（W）
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    if trace_request == 0 { 
        let va=VirtAddr::from(id);
        let vpn=va.floor();
        let offset=va.page_offset();

        let pte=match translate_vpn_to_pte(vpn){
            Some(pte)=>pte,
            None=>return -1
        };
        
        // 检查页面是否可访问
        if !pte.is_valid()|| !pte.user_accessible(){
            return -1;
        }

        // 检查是否可读
        if !pte.readable(){
            return -1;
        }
        // 开始读取
        let ppn=pte.ppn();
        let src=ppn.get_bytes_array();
        src[offset] as isize
    }else if trace_request==1{
        let va=VirtAddr::from(id);
        let vpn=va.floor();
        let offset=va.page_offset();

        // 获取当前页表项
        let pte=match translate_vpn_to_pte(vpn){
            Some(pte)=>pte,
            None=>return -1
        };
        if !pte.is_valid()|| !pte.user_accessible(){
            return -1;
        }
        if !pte.writable(){
            return -1;
        }
        let ppn=pte.ppn();
        let dst=ppn.get_bytes_array();
        dst[offset]=data as u8;
        0
    }else if trace_request==2{
        get_syscall_count(id) as isize
    }else{
        return -1;
    }
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
    
    current_momory_set(|memory_set|{
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
    current_momory_set(|memory_set|{
        if memory_set.munmap(VirtAddr::from(start),len){
            0
        }else{
            -1
        }
    })
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
