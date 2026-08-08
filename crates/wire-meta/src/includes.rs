use serde::{Deserialize, Serialize};

pub trait Includable {}

/// Unit include type for entities that don't support includes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NoInclude {}

impl Includable for NoInclude {}

// TODO validate max length on includes
pub trait HasIncludes<T>
where
    T: Includable,
{
    fn includes(&mut self) -> &mut Option<Vec<T>>;
    fn with_include(mut self, include: T) -> Self
    where
        Self: Sized,
    {
        let includes = self.includes();
        if includes.is_none() {
            *includes = Some(Vec::new());
        }
        includes.as_mut().unwrap().push(include);
        self
    }
}
