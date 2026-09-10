//! 環境変数を変更するテスト用の直列化ロックと RAII ヘルパ。
//!
//! Rust 1.70 以降、`std::env::{set_var, remove_var}` はプロセス全体の環境を
//! 裏で `RefCell` として扱うため、複数スレッド（並列 `cargo test`）で同時に
//! 呼び出すとデータ競合で断言が外れたり panic になる。本モジュールは全 env 変更
//! テストを 1 つのグローバルロックで直列化し、env 操作をヘルパに集約して
//! 元の値を確実に復元する。

use std::ffi::OsStr;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// プロセス 1 箇所に 1 つの env 変更テストだけを許可するロック。
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// env 変更セクションを直列化するロックを獲得する。
///
/// 返した guard を生かす限り他スレッドの env 変更はブロックされる。
/// 関数本体の冒頭で獲得し、関数終端で自然に drop して解放する。
#[allow(clippy::missing_panics_doc)]
pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
    let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
    lock.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// 環境変数 1 件を `value` に設定したまま閉包を実行し、スコープ終了で元に戻す。
///
/// 設定・復元・実行はロック保持下で 1 連なり、並列 `cargo test` で他テストの env
/// 変更と競合しない。assert 内容は閉包内にあるため値の弱体化にならない。
pub(crate) fn with_env<K, F, V>(name: &str, value: V, f: F) -> K
where
    F: FnOnce() -> K,
    V: AsRef<OsStr>,
{
    let guard = lock_env();
    let prev = std::env::var_os(name);
     // SAFETY: 本ロック保持下で 1 連なり。他スレッドの env 変更はブロック済み。
    unsafe { std::env::set_var(name, value) };
    let out = f();
    match prev {
        Some(v) => unsafe { std::env::set_var(name, v) },
        None => unsafe { std::env::remove_var(name) },
      }
    drop(guard);
    out
}

/// 環境変数群をまとめて remove したまま閉包を実行し、スコープ終了で元に戻す。
///
/// 2 件以上の env を取り除くテスト（例: 新旧の key 名両方）に使う。
/// nested な without_env は std Mutex の非可再入でデッドロックするため
/// 複数 key は本関数で一括処理する。
pub(crate) fn without_all<K, F>(names: &[&str], f: F) -> K
where
    F: FnOnce() -> K,
{
    let guard = lock_env();
    let prev: Vec<(String, Option<std::ffi::OsString>)> =
         names.iter().map(|n| (n.to_string(), std::env::var_os(n))).collect();
    for (name, _) in &prev {
        // SAFETY: ロック保持下で 1 連なり。
        unsafe { std::env::remove_var(name) };
      }
    let out = f();
    for (name, val) in &prev {
        match val {
            Some(v) => unsafe { std::env::set_var(name, v) },
            None => {}
          }
      }
    drop(guard);
    out
}
