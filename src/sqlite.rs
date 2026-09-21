use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

type Destructor = Option<unsafe extern "C" fn(*mut c_void)>;
// SQLITE_STATIC: bind strings live until finalize in the same call.
const SQLITE_STATIC: Destructor = None;

#[repr(C)]
struct sqlite3 {
    _private: [u8; 0],
}
#[repr(C)]
struct sqlite3_stmt {
    _private: [u8; 0],
}

#[link(name = "sqlite3")]
extern "C" {
    fn sqlite3_open(filename: *const c_char, pp_db: *mut *mut sqlite3) -> c_int;
    fn sqlite3_close(db: *mut sqlite3) -> c_int;
    fn sqlite3_exec(
        db: *mut sqlite3,
        sql: *const c_char,
        cb: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(p: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        z_sql: *const c_char,
        n_byte: c_int,
        pp_stmt: *mut *mut sqlite3_stmt,
        pz_tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        idx: c_int,
        val: *const c_char,
        n: c_int,
        destructor: Destructor,
    ) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, idx: c_int, val: i64) -> c_int;
    fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_column_text(stmt: *mut sqlite3_stmt, i_col: c_int) -> *const u8;
    fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, i_col: c_int) -> i64;
    fn sqlite3_column_bytes(stmt: *mut sqlite3_stmt, i_col: c_int) -> c_int;
    fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
}

pub enum Bind<'a> {
    Text(&'a str),
    I64(i64),
}

pub struct Connection {
    db: *mut sqlite3,
}

unsafe impl Send for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.db);
        }
    }
}

impl Connection {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_string_lossy();
        let c_path = CString::new(path.as_ref()).map_err(|_| "db path")?;
        let mut db = ptr::null_mut();
        let rc = unsafe { sqlite3_open(c_path.as_ptr(), &mut db) };
        if rc != SQLITE_OK {
            let msg = errmsg(db);
            unsafe { sqlite3_close(db) };
            return Err(msg);
        }
        Ok(Self { db })
    }

    pub fn execute_batch(&self, sql: &str) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|_| "sql")?;
        let mut err = ptr::null_mut();
        let rc = unsafe {
            sqlite3_exec(
                self.db,
                c_sql.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                &mut err,
            )
        };
        if rc != SQLITE_OK {
            let msg = if err.is_null() {
                errmsg(self.db)
            } else {
                let s = unsafe { CStr::from_ptr(err) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { sqlite3_free(err as *mut c_void) };
                s
            };
            return Err(msg);
        }
        Ok(())
    }

    pub fn execute(&self, sql: &str, binds: &[Bind<'_>]) -> Result<usize, String> {
        let stmt = self.prepare(sql, binds)?;
        let rc = unsafe { sqlite3_step(stmt) };
        let changes = unsafe { sqlite3_changes(self.db) } as usize;
        unsafe { sqlite3_finalize(stmt) };
        if rc != SQLITE_DONE && rc != SQLITE_ROW {
            return Err(errmsg(self.db));
        }
        Ok(changes)
    }

    pub fn query_row<T>(
        &self,
        sql: &str,
        binds: &[Bind<'_>],
        f: impl FnOnce(&Row) -> T,
    ) -> Result<Option<T>, String> {
        let stmt = self.prepare(sql, binds)?;
        let rc = unsafe { sqlite3_step(stmt) };
        let out = if rc == SQLITE_ROW {
            Some(f(&Row { stmt }))
        } else if rc == SQLITE_DONE {
            None
        } else {
            unsafe { sqlite3_finalize(stmt) };
            return Err(errmsg(self.db));
        };
        unsafe { sqlite3_finalize(stmt) };
        Ok(out)
    }

    pub fn query<T>(
        &self,
        sql: &str,
        binds: &[Bind<'_>],
        mut f: impl FnMut(&Row) -> T,
    ) -> Result<Vec<T>, String> {
        let stmt = self.prepare(sql, binds)?;
        let mut rows = Vec::new();
        loop {
            let rc = unsafe { sqlite3_step(stmt) };
            if rc == SQLITE_ROW {
                rows.push(f(&Row { stmt }));
            } else if rc == SQLITE_DONE {
                break;
            } else {
                unsafe { sqlite3_finalize(stmt) };
                return Err(errmsg(self.db));
            }
        }
        unsafe { sqlite3_finalize(stmt) };
        Ok(rows)
    }

    /// Start a write transaction. `BEGIN IMMEDIATE` takes the write lock up
    /// front, so a read-then-write sequence cannot be interleaved by another
    /// writer. Dropping the guard without `commit` rolls back, including when
    /// the caller returns early with `?` or panics.
    pub fn begin(&self) -> Result<Transaction<'_>, String> {
        self.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Transaction {
            conn: self,
            open: true,
        })
    }

    fn prepare(&self, sql: &str, binds: &[Bind<'_>]) -> Result<*mut sqlite3_stmt, String> {
        let c_sql = CString::new(sql).map_err(|_| "sql")?;
        let mut stmt = ptr::null_mut();
        let rc =
            unsafe { sqlite3_prepare_v2(self.db, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut()) };
        if rc != SQLITE_OK {
            return Err(errmsg(self.db));
        }
        for (i, bind) in binds.iter().enumerate() {
            let idx = (i + 1) as c_int;
            let rc = match bind {
                Bind::Text(s) => unsafe {
                    sqlite3_bind_text(
                        stmt,
                        idx,
                        s.as_ptr() as *const c_char,
                        s.len() as c_int,
                        SQLITE_STATIC,
                    )
                },
                Bind::I64(n) => unsafe { sqlite3_bind_int64(stmt, idx, *n) },
            };
            if rc != SQLITE_OK {
                unsafe { sqlite3_finalize(stmt) };
                return Err(errmsg(self.db));
            }
        }
        Ok(stmt)
    }
}

pub struct Transaction<'a> {
    conn: &'a Connection,
    open: bool,
}

impl Transaction<'_> {
    pub fn commit(mut self) -> Result<(), String> {
        let done = self.conn.execute_batch("COMMIT");
        // A failed COMMIT (disk full, busy) leaves the transaction open;
        // Drop rolls it back.
        if done.is_ok() {
            self.open = false;
        }
        done
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

pub struct Row {
    stmt: *mut sqlite3_stmt,
}

impl Row {
    pub fn text(&self, i: i32) -> String {
        unsafe {
            let ptr = sqlite3_column_text(self.stmt, i);
            if ptr.is_null() {
                return String::new();
            }
            let n = sqlite3_column_bytes(self.stmt, i) as usize;
            let slice = std::slice::from_raw_parts(ptr, n);
            String::from_utf8_lossy(slice).into_owned()
        }
    }

    pub fn i64(&self, i: i32) -> i64 {
        unsafe { sqlite3_column_int64(self.stmt, i) }
    }
}

fn errmsg(db: *mut sqlite3) -> String {
    if db.is_null() {
        return "sqlite error".into();
    }
    unsafe { CStr::from_ptr(sqlite3_errmsg(db)) }
        .to_string_lossy()
        .into_owned()
}
