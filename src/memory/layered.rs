//! 複数 [`MemoryBridge`] を重ねる（local を外部で置き換えない）。

use super::{DiaryEntry, MemoryBridge, MemoryError, RecalledItem};

/// 先頭レイヤが優先（通常は local）。読み取りは結合、書き込みは全レイヤへ。
pub struct LayeredMemoryBridge {
    layers: Vec<Box<dyn MemoryBridge>>,
}

impl LayeredMemoryBridge {
    pub fn new(layers: Vec<Box<dyn MemoryBridge>>) -> Self {
        Self { layers }
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

impl MemoryBridge for LayeredMemoryBridge {
    fn recent_work(&self, max_entries: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        let mut out = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            match layer.recent_work(max_entries) {
                Ok(items) => out.extend(items),
                Err(err) => {
                    eprintln!("[memory] recent_work layer[{i}]: {err}");
                }
            }
        }
        Ok(out)
    }

    fn search(&self, query: &str, top_k: usize) -> Result<Vec<RecalledItem>, MemoryError> {
        let mut out = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            match layer.search(query, top_k) {
                Ok(items) => out.extend(items),
                Err(err) => {
                    eprintln!("[memory] search layer[{i}]: {err}");
                }
            }
        }
        Ok(out)
    }

    fn diary(&mut self, entry: &DiaryEntry) -> Result<(), MemoryError> {
        let mut any_ok = self.layers.is_empty();
        let mut last_err: Option<MemoryError> = None;
        for (i, layer) in self.layers.iter_mut().enumerate() {
            match layer.diary(entry) {
                Ok(()) => any_ok = true,
                Err(err) => {
                    eprintln!("[memory] diary layer[{i}]: {err}");
                    last_err = Some(err);
                }
            }
        }
        if any_ok {
            Ok(())
        } else {
            Err(last_err.unwrap_or_else(|| {
                MemoryError::Backend("all memory layers failed diary write".into())
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DiaryPhase, LocalDiaryBridge, RecalledSource};

    struct FailBridge;

    impl MemoryBridge for FailBridge {
        fn recent_work(&self, _: usize) -> Result<Vec<RecalledItem>, MemoryError> {
            Err(MemoryError::Backend("down".into()))
        }
        fn search(&self, _: &str, _: usize) -> Result<Vec<RecalledItem>, MemoryError> {
            Err(MemoryError::Backend("down".into()))
        }
        fn diary(&mut self, _: &DiaryEntry) -> Result<(), MemoryError> {
            Err(MemoryError::Backend("down".into()))
        }
    }

    #[test]
    fn local_survives_when_external_fails() {
        let mut local = LocalDiaryBridge::new();
        local
            .diary(&DiaryEntry {
                user_input: "session work".into(),
                summary: "s".into(),
                answer: "a".into(),
                phases: vec![DiaryPhase {
                    id: 1,
                    goal: "g".into(),
                    answer: "ok".into(),
                }],
            })
            .unwrap();
        let mut layered = LayeredMemoryBridge::new(vec![
            Box::new(local),
            Box::new(FailBridge),
        ]);
        let recent = layered.recent_work(3).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].source, RecalledSource::RecentWork);
        layered
            .diary(&DiaryEntry {
                user_input: "next".into(),
                summary: "s2".into(),
                answer: "a2".into(),
                phases: vec![],
            })
            .unwrap();
        assert_eq!(layered.recent_work(3).unwrap().len(), 2);
    }
}
