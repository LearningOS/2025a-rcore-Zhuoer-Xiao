use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // Check for deadlock if detection is enabled
    if process_inner.deadlock_detection_enabled {
        // Deadlock detection for mutex
        if detect_deadlock_mutex(mutex_id) {
            drop(process_inner);
            return -0xDEAD as isize;
        }
    }
    
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.lock();
    
    // 记录获取的互斥锁
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    if !task_inner.held_mutexes.contains(&mutex_id) {
        task_inner.held_mutexes.push(mutex_id);
    }
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // 从任务中移除已释放的互斥锁
    let task = current_task().unwrap();
    {
        let mut task_inner = task.inner_exclusive_access();
        task_inner.held_mutexes.retain(|&id| id != mutex_id);
    }
    
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // 从任务中移除已释放的信号量
    let task = current_task().unwrap();
    {
        let mut task_inner = task.inner_exclusive_access();
        task_inner.held_semaphores.retain(|&id| id != sem_id);
    }
    
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // Check for deadlock if detection is enabled
    if process_inner.deadlock_detection_enabled {
        // Deadlock detection for semaphore
        if detect_deadlock_semaphore(sem_id) {
            drop(process_inner);
            return -0xDEAD as isize;
        }
    }
    
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.down();
    
    // 记录获取的信号量
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    if !task_inner.held_semaphores.contains(&sem_id) {
        task_inner.held_semaphores.push(sem_id);
    }
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_enable_deadlock_detect",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    
    // 参数检查
    if enabled != 0 && enabled != 1 {
        return -1;
    }
    
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detection_enabled = enabled == 1;
    0
}

// 死锁检测函数 - 检查互斥锁
fn detect_deadlock_mutex(requested_mutex_id: usize) -> bool {
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // 获取进程总数和资源类型数
    let num_processes = process_inner.tasks.len();
    let num_mutexes = process_inner.mutex_list.len();
    
    // 初始化资源向量和矩阵
    let mut available = vec![1; num_mutexes]; // 每个互斥锁只有一个实例
    let mut allocation = vec![vec![0; num_mutexes]; num_processes];
    let mut need = vec![vec![0; num_mutexes]; num_processes];
    
    // 更新当前已分配的资源
    for (tid, task_opt) in process_inner.tasks.iter().enumerate() {
        if let Some(task) = task_opt {
            let task = task.as_ref();
            let task_inner = task.inner_exclusive_access();
            // 计算allocation矩阵
            for &mutex_id in &task_inner.held_mutexes {
                if mutex_id < num_mutexes {
                    allocation[tid][mutex_id] = 1;
                    available[mutex_id] = 0; // 标记为已分配
                }
            }
            drop(task_inner);
        }
    }
    
    // 计算need矩阵 (所有未获取的mutex都需要)
    for tid in 0..num_processes {
        for mutex_id in 0..num_mutexes {
            // 如果还没有获得这个mutex，则need为1
            if process_inner.mutex_list[mutex_id].is_some() {
                need[tid][mutex_id] = 1;
            }
        }
    }
    
    // 减去已分配的资源
    for tid in 0..num_processes {
        for mutex_id in 0..num_mutexes {
            need[tid][mutex_id] -= allocation[tid][mutex_id];
        }
    }
    
    // 当前进程请求的互斥锁
    let current_tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    if current_tid < num_processes && process_inner.mutex_list[requested_mutex_id].is_some() {
        need[current_tid][requested_mutex_id] = 1;
    }
    
    drop(process_inner);
    
    // 银行家算法安全性检查
    is_safe_state(num_processes, num_mutexes, &available, &allocation, &need)
}

// 死锁检测函数 - 检查信号量
fn detect_deadlock_semaphore(requested_sem_id: usize) -> bool {
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    
    // 获取进程总数和资源类型数
    let num_processes = process_inner.tasks.len();
    let num_sems = process_inner.semaphore_list.len();
    
    // 初始化资源向量和矩阵
    // 注意：信号量可以有多个实例，需要获取每个信号量的可用数量
    let mut available = vec![0; num_sems];
    let mut allocation = vec![vec![0; num_sems]; num_processes];
    let mut need = vec![vec![0; num_sems]; num_processes];
    
    // 获取每个信号量的可用数量
    for (sem_id, sem_opt) in process_inner.semaphore_list.iter().enumerate() {
        if let Some(sem) = sem_opt {
            available[sem_id] = sem.available();
        }
    }
    
    // 更新当前已分配的资源
    for (tid, task_opt) in process_inner.tasks.iter().enumerate() {
        if let Some(task) = task_opt {
            let task = task.as_ref();
            let task_inner = task.inner_exclusive_access();
            // 计算allocation矩阵
            for &sem_id in &task_inner.held_semaphores {
                if sem_id < num_sems {
                    allocation[tid][sem_id] += 1;
                }
            }
            drop(task_inner);
        }
    }
    
    // 计算need矩阵 (所有未获取的semaphore都需要)
    for tid in 0..num_processes {
        for sem_id in 0..num_sems {
            // 如果还没有获得这个semaphore，则need为1
            if process_inner.semaphore_list[sem_id].is_some() {
                need[tid][sem_id] = 1;
            }
        }
    }
    
    // 减去已分配的资源
    for tid in 0..num_processes {
        for sem_id in 0..num_sems {
            need[tid][sem_id] -= allocation[tid][sem_id];
        }
    }
    
    // 当前进程请求的信号量
    let current_tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    if current_tid < num_processes && process_inner.semaphore_list[requested_sem_id].is_some() {
        need[current_tid][requested_sem_id] = 1;
    }
    
    drop(process_inner);
    
    // 银行家算法安全性检查
    is_safe_state(num_processes, num_sems, &available, &allocation, &need)
}

// 银行家算法实现：检查当前状态是否安全
fn is_safe_state(num_processes: usize, num_resources: usize, available: &[usize], allocation: &[Vec<usize>], need: &[Vec<usize>]) -> bool {
    let mut work = available.to_vec();
    let mut finish = vec![false; num_processes];
    
    loop {
        let mut found = false;
        
        // 寻找满足条件的进程
        for i in 0..num_processes {
            if !finish[i] && (0..num_resources).all(|j| need[i][j] <= work[j]) {
                // 找到满足条件的进程
                for j in 0..num_resources {
                    work[j] += allocation[i][j];
                }
                finish[i] = true;
                found = true;
                break;
            }
        }
        
        // 如果没有找到可执行的进程
        if !found {
            break;
        }
    }
    
    // 检查是否所有进程都能完成
    finish.iter().all(|&f| f)
}