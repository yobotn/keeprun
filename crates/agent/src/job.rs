//! Windows Job Objects and process spawning.
//!
//! Every app runs in its own job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The child is
//! created suspended, assigned to the job, then resumed, so no grandchild can escape the job.
//! Closing the job handle (or the agent exiting) kills the whole process tree.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::JobObjects::{
    JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList, QueryInformationJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ,
};
use windows::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, THREAD_SUSPEND_RESUME,
};

/// Owned job handle. Dropping it kills every process still in the job.
pub struct Job(HANDLE);

// A job handle is a kernel object handle, safe to use from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> Result<Self> {
        unsafe {
            let h = CreateJobObjectW(None, PCWSTR::null()).context("CreateJobObjectW failed")?;
            let job = Job(h);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .context("SetInformationJobObject failed")?;
            Ok(job)
        }
    }

    fn assign(&self, child: &Child) -> Result<()> {
        let Some(raw) = child.raw_handle() else {
            bail!("The process exited before it could be added to its job.");
        };
        unsafe {
            AssignProcessToJobObject(self.0, HANDLE(raw as _))
                .context("AssignProcessToJobObject failed")
        }
    }

    /// Kill every process in the job.
    pub fn terminate(&self, exit_code: u32) {
        unsafe {
            let _ = TerminateJobObject(self.0, exit_code);
        }
    }

    /// Ids of every process currently in the job.
    pub fn process_ids(&self) -> Vec<u32> {
        // Header is two u32 counters, then a ULONG_PTR array.
        let mut buf = vec![0usize; 2 + 1024];
        unsafe {
            let ok = QueryInformationJobObject(
                Some(self.0),
                JobObjectBasicProcessIdList,
                buf.as_mut_ptr() as *mut _,
                (buf.len() * std::mem::size_of::<usize>()) as u32,
                None,
            );
            if ok.is_err() {
                return vec![];
            }
            let list = &*(buf.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST);
            let n = list.NumberOfProcessIdsInList as usize;
            let ids = std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), n);
            ids.iter().map(|&p| p as u32).collect()
        }
    }

    /// Total user plus kernel CPU time of all processes ever in the job, in 100 ns units.
    pub fn cpu_time(&self) -> u64 {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            let ok = QueryInformationJobObject(
                Some(self.0),
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            );
            if ok.is_err() {
                return 0;
            }
        }
        (info.TotalUserTime + info.TotalKernelTime) as u64
    }
}

/// Working set of a process in bytes, or 0 when it cannot be read.
pub fn memory_of(pid: u32) -> u64 {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return 0;
        };
        let mut c = PROCESS_MEMORY_COUNTERS::default();
        let ok = GetProcessMemoryInfo(
            h,
            &mut c,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        let _ = CloseHandle(h);
        if ok.is_ok() {
            c.WorkingSetSize as u64
        } else {
            0
        }
    }
}

/// Executable file name of a process, e.g. "node.exe".
pub fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

/// Every process on the machine as (pid, parent pid, executable file name).
pub fn processes() -> Vec<(u32, u32, String)> {
    let mut out = vec![];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            out.push((
                e.th32ProcessID,
                e.th32ParentProcessID,
                String::from_utf16_lossy(&e.szExeFile[..len]),
            ));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: u32,
        info: *mut std::ffi::c_void,
        len: u32,
        ret: *mut u32,
    ) -> i32;
}

/// `ProcessCommandLineInformation`, Windows 8.1 and later.
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

/// What a process runs and from where.
#[derive(Clone)]
pub struct ProcInfo {
    pub cmd: String,
    /// Start time in FILETIME ticks.
    pub started: u64,
    /// Current folder, when it can be read.
    pub cwd: Option<String>,
}

/// Command line, start time and current folder of a process, or None when it cannot be read,
/// e.g. a protected system process.
pub fn process_info(pid: u32) -> Option<ProcInfo> {
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        max: u16,
        buffer: *const u16,
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = Default::default();
        let times = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
        // The longest command line is 32767 UTF-16 units, after a UNICODE_STRING header.
        let mut buf = vec![0u64; 8200];
        let mut ret = 0u32;
        let status = NtQueryInformationProcess(
            h,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut ret,
        );
        let _ = CloseHandle(h);
        if status < 0 || times.is_err() {
            return None;
        }
        let s = &*(buf.as_ptr() as *const UnicodeString);
        let cmd = if s.buffer.is_null() {
            String::new()
        } else {
            String::from_utf16_lossy(std::slice::from_raw_parts(s.buffer, s.length as usize / 2))
        };
        Some(ProcInfo {
            cmd,
            started: (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64,
            cwd: current_dir(pid),
        })
    }
}

/// `ProcessBasicInformation` and `ProcessWow64Information`.
const PROCESS_BASIC_INFORMATION: u32 = 0;
const PROCESS_WOW64_INFORMATION: u32 = 26;

/// Current folder of a 64-bit process, read from its process parameters. There is no API
/// for it: `npm start` runs `node server.js`, so this is how a relative script is placed.
fn current_dir(pid: u32) -> Option<String> {
    #[repr(C)]
    #[derive(Default)]
    struct BasicInfo {
        exit_status: isize,
        peb: usize,
        affinity: usize,
        base_priority: isize,
        pid: usize,
        parent: usize,
    }
    unsafe fn read<T: Default>(h: HANDLE, addr: usize) -> Option<T> {
        let mut v = T::default();
        ReadProcessMemory(
            h,
            addr as *const std::ffi::c_void,
            &mut v as *mut T as *mut std::ffi::c_void,
            std::mem::size_of::<T>(),
            None,
        )
        .ok()?;
        Some(v)
    }
    if cfg!(not(target_pointer_width = "64")) {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let dir = (|| {
            // A 32-bit process lays its parameters out differently.
            let mut wow = 0usize;
            let st = NtQueryInformationProcess(
                h,
                PROCESS_WOW64_INFORMATION,
                (&mut wow as *mut usize).cast(),
                std::mem::size_of::<usize>() as u32,
                std::ptr::null_mut(),
            );
            if st < 0 || wow != 0 {
                return None;
            }
            let mut bi = BasicInfo::default();
            let st = NtQueryInformationProcess(
                h,
                PROCESS_BASIC_INFORMATION,
                (&mut bi as *mut BasicInfo).cast(),
                std::mem::size_of::<BasicInfo>() as u32,
                std::ptr::null_mut(),
            );
            if st < 0 || bi.peb == 0 {
                return None;
            }
            // PEB.ProcessParameters, then its CurrentDirectory.DosPath (a UNICODE_STRING).
            let params: usize = read(h, bi.peb + 0x20)?;
            let len: u16 = read(h, params + 0x38)?;
            let buffer: usize = read(h, params + 0x40)?;
            if buffer == 0 || len == 0 {
                return None;
            }
            let mut text = vec![0u16; len as usize / 2];
            ReadProcessMemory(
                h,
                buffer as *const std::ffi::c_void,
                text.as_mut_ptr().cast(),
                len as usize,
                None,
            )
            .ok()?;
            Some(String::from_utf16_lossy(&text))
        })();
        let _ = CloseHandle(h);
        dir
    }
}

/// End processes, then wait up to 3 seconds for each to exit so its ports are free.
pub fn kill(pids: &[u32]) {
    let mut handles = vec![];
    unsafe {
        for &pid in pids {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
                let _ = TerminateProcess(h, 1);
                handles.push(h);
            }
        }
        for h in handles {
            WaitForSingleObject(h, 3000);
            let _ = CloseHandle(h);
        }
    }
}

/// Every listening TCP socket on the machine as (owning pid, port), IPv4 and IPv6.
pub fn listening_ports() -> Vec<(u32, u16)> {
    let mut out = vec![];
    for family in [AF_INET.0 as u32, AF_INET6.0 as u32] {
        let mut size = 0u32;
        unsafe {
            let _ = GetExtendedTcpTable(
                None,
                &mut size,
                false,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
            if size == 0 {
                continue;
            }
            let mut buf = vec![0u8; size as usize + 1024];
            size = buf.len() as u32;
            if GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut _),
                &mut size,
                false,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            ) != 0
            {
                continue;
            }
            let port = |p: u32| u16::from_be(p as u16);
            if family == AF_INET.0 as u32 {
                let t = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
                let rows = std::slice::from_raw_parts(t.table.as_ptr(), t.dwNumEntries as usize);
                out.extend(rows.iter().map(|r| (r.dwOwningPid, port(r.dwLocalPort))));
            } else {
                let t = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
                let rows = std::slice::from_raw_parts(t.table.as_ptr(), t.dwNumEntries as usize);
                out.extend(rows.iter().map(|r| (r.dwOwningPid, port(r.dwLocalPort))));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Resume every thread of a process that was created with `CREATE_SUSPENDED`.
fn resume_process(pid: u32) -> Result<()> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
            .context("CreateToolhelp32Snapshot failed")?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut resumed = 0;
        let mut ok = Thread32First(snap, &mut entry).is_ok();
        while ok {
            if entry.th32OwnerProcessID == pid {
                if let Ok(t) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) {
                    ResumeThread(t);
                    let _ = CloseHandle(t);
                    resumed += 1;
                }
            }
            ok = Thread32Next(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        if resumed == 0 {
            bail!("Could not resume process {pid}.");
        }
        Ok(())
    }
}

/// Spawn `command` through `cmd.exe` inside a new job. Returns the child and its job.
/// The app port from its PORT variable, if set to a valid number.
pub fn port_of(env: &BTreeMap<String, String>) -> Option<u16> {
    env.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PORT"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .filter(|p| *p != 0)
}

const NO_CWD_IN_PATH: &str = "NoDefaultCurrentDirectoryInExePath";

pub fn spawn_in_job(
    cwd: &Path,
    command: &str,
    env: &BTreeMap<String, String>,
) -> Result<(Child, Job)> {
    if !cwd.is_dir() {
        bail!(
            "Folder {} does not exist. Check the app folder in Settings.",
            cwd.display()
        );
    }
    let job = Job::new()?;
    let mut cmd = Command::new("cmd.exe");
    cmd.raw_arg(format!("/d /s /c \"{command}\""))
        .current_dir(cwd);
    // Hardened servers set this to stop cmd.exe finding programs in the current folder.
    // Commands like `start.bat` must resolve inside the app folder the admin chose.
    if !env.keys().any(|k| k.eq_ignore_ascii_case(NO_CWD_IN_PATH)) {
        cmd.env_remove(NO_CWD_IN_PATH);
    }
    cmd.envs(env);
    // ASP.NET Core ignores PORT. Map it, unless the app sets its own URLs.
    if let Some(port) = port_of(env) {
        let has = |n: &str| env.keys().any(|k| k.eq_ignore_ascii_case(n));
        if ![
            "ASPNETCORE_URLS",
            "ASPNETCORE_HTTP_PORTS",
            "ASPNETCORE_HTTPS_PORTS",
            "URLS",
        ]
        .iter()
        .any(|n| has(n))
        {
            cmd.env("ASPNETCORE_URLS", format!("http://*:{port}"));
        }
    }
    if let Some(path) = path_with_tool_dir(command, env) {
        cmd.env("PATH", path);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(
            (CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT).0,
        );
    let mut child = cmd
        .spawn()
        .with_context(|| format!("Cannot start \"{command}\""))?;
    let pid = child.id().unwrap_or(0);
    if let Err(e) = job.assign(&child).and_then(|_| resume_process(pid)) {
        // The child may not be in the job, so end it directly too.
        let _ = child.start_kill();
        job.terminate(1);
        return Err(e);
    }
    Ok((child, job))
}

/// When a command starts with an absolute executable path (as the detector writes them),
/// put that folder first on PATH. `npm.cmd` then finds `node.exe` even when the agent runs as
/// LocalSystem, whose PATH does not include per-user installs.
fn path_with_tool_dir(command: &str, env: &BTreeMap<String, String>) -> Option<String> {
    let first = command.trim_start();
    let exe = if let Some(rest) = first.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        first.split_whitespace().next()?
    };
    let dir = Path::new(exe)
        .parent()
        .filter(|d| d.is_absolute() && d.is_dir())?;
    let base = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    Some(format!("{};{base}", dir.display()))
}

/// Ask the process group to stop, like pressing Ctrl+Break in its console.
pub fn send_ctrl_break(pid: u32) -> bool {
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid).is_ok() }
}

/// Services have no console, so Ctrl+Break cannot be delivered. Give the agent a hidden one
/// that child processes share.
pub fn ensure_hidden_console() {
    use windows::Win32::System::Console::{AllocConsole, GetConsoleWindow};
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        if AllocConsole().is_ok() {
            let w = GetConsoleWindow();
            if !w.is_invalid() {
                let _ = ShowWindow(w, SW_HIDE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// A script in the app folder runs even when the agent inherited
    /// NoDefaultCurrentDirectoryInExePath (set on hardened servers and some shells).
    #[tokio::test]
    async fn port_is_passed_to_aspnet_core() {
        let env = BTreeMap::from([("PORT".to_string(), "4567".to_string())]);
        let (mut child, _job) =
            spawn_in_job(&std::env::temp_dir(), "echo %ASPNETCORE_URLS%", &env).unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .await
            .unwrap();
        child.wait().await.unwrap();
        assert_eq!(out.trim(), "http://*:4567");
    }

    #[tokio::test]
    async fn runs_script_from_app_folder() {
        let dir = std::env::temp_dir().join(format!("keeprun-job-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.cmd"), "@echo from-folder\r\n").unwrap();
        let (mut child, _job) = spawn_in_job(&dir, "run.cmd", &BTreeMap::new()).unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .await
            .unwrap();
        let status = child.wait().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(status.success(), "exit {status:?}");
        assert!(out.contains("from-folder"), "output: {out}");
    }
}
