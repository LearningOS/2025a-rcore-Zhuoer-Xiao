# ch4实验代码
>1.在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：  
未与他人交流  
2.此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：
《RUST语言圣经》 、通义千问语法搜索等。  
3.我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。  
4.我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。
## 实验设计与代码

## 简答作业
在我们的多线程实现中，当主线程 (即 0 号线程) 退出时，视为整个进程退出， 此时需要结束该进程管理的所有线程并回收其资源。 - 需要回收的资源有哪些？ - 其他线程的 TaskControlBlock 可能在哪些位置被引用，分别是否需要回收，为什么？  
>所有线程的内核栈（Kernel Stack）：每个线程都有独立的内核栈，需要释放
所有线程的任务控制块（TaskControlBlock）：包含线程的所有状态信息
线程的用户资源（TaskUserRes）：包括用户栈等用户态资源
进程控制块（ProcessControlBlock）：包含进程的内存空间等信息
进程的内存地址空间（MemorySet）：包括代码段、数据段、堆、栈等
文件描述符表及相关资源：如果进程打开了文件，需要关闭并释放相关资源
信号量、互斥锁等同步原语：如果进程创建了这些资源，需要清理
进程和线程在各种管理结构中的引用：如任务管理器中的就绪队列等
其他线程的 TaskControlBlock 可能在以下位置被引用：

>任务管理器（TaskManager）的就绪队列：
TASK_MANAGER 中的 ready_queue
进程退出时，属于该进程的所有线程都应该被终止，不再参与调度
等待队列：
各种同步原语（如互斥锁、信号量）的等待队列
进程退出时，线程不应继续等待资源，应从所有等待队列中移除
父子进程关系：
父进程的 children 字段，子进程的 parent 字段
需要清理进程间的父子关系，避免悬垂指针
文件系统相关结构：
打开的文件描述符表等
进程退出时需要关闭所有打开的文件
内存管理结构：
页表项、物理页帧管理器等
进程的地址空间需要被完全释放，避免内存泄漏

对比以下两种 Mutex 中的实现，二者有什么区别？这些区别可能会导致什么问题？
``` rust
impl Mutex for Mutex1 {
 2    fn lock(&self) {
 3        loop {
 4            let mut mutex_inner = self.inner.exclusive_access();
 5            if mutex_inner.locked {
 6                mutex_inner.wait_queue.push_back(current_task().unwrap());
 7                drop(mutex_inner);
 8                block_current_and_run_next();
 9            } else {
10                mutex_inner.locked = true;
11                break;
12            }
13        }
14    }
15
16    fn unlock(&self) {
17        let mut mutex_inner = self.inner.exclusive_access();
18        assert!(mutex_inner.locked);
19        mutex_inner.locked = false;
20        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
21            add_task(waking_task);
22        }
23    }
24}
25
26impl Mutex for Mutex2 {
27    fn lock(&self) {
28        let mut mutex_inner = self.inner.exclusive_access();
29        if mutex_inner.locked {
30            mutex_inner.wait_queue.push_back(current_task().unwrap());
31            drop(mutex_inner);
32            block_current_and_run_next();
33        } else {
34            mutex_inner.locked = true;
35        }
36    }
37
38    fn unlock(&self) {
39        let mut mutex_inner = self.inner.exclusive_access();
40        assert!(mutex_inner.locked);
41        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
42            add_task(waking_task);
43        } else {
44            mutex_inner.locked = false;
45        }
46    }
47}
```
>Mutex1：
在 lock 方法中使用了循环，会持续检查锁状态，直到获取到锁
在 unlock 方法中，无论是否有等待的线程，都会将 locked 设置为 false  
Mutex2：
在 lock 方法中没有循环，如果第一次检查发现锁被占用，就直接进入等待状态
在 unlock 方法中，只有在没有等待线程时才将 locked 设置为 false
当线程被唤醒后（从 block_current_and_run_next 返回），不会再次检查锁是否可用就直接返回
这可能导致多个线程同时认为自己获取了锁，破坏了互斥性