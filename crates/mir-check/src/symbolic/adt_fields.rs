use super::Value;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct AdtFields(Rc<Vec<(String, Value)>>);

impl Deref for AdtFields {
    type Target = Vec<(String, Value)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AdtFields {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Rc::make_mut(&mut self.0)
    }
}

impl From<Vec<(String, Value)>> for AdtFields {
    fn from(fields: Vec<(String, Value)>) -> Self {
        Self(Rc::new(fields))
    }
}

impl FromIterator<(String, Value)> for AdtFields {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(values: T) -> Self {
        Vec::from_iter(values).into()
    }
}

impl IntoIterator for AdtFields {
    type Item = (String, Value);
    type IntoIter = std::vec::IntoIter<(String, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        Rc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<'a> IntoIterator for &'a AdtFields {
    type Item = &'a (String, Value);
    type IntoIter = std::slice::Iter<'a, (String, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut AdtFields {
    type Item = &'a mut (String, Value);
    type IntoIter = std::slice::IterMut<'a, (String, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        Rc::make_mut(&mut self.0).iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbolic::{Context, integer};

    fn record(fields: Vec<(String, Value)>) -> Value {
        Value::Adt {
            name: "Record".into(),
            variant: 0,
            is_option: false,
            discriminant: 0,
            fields: fields.into(),
        }
    }

    fn fields(value: &Value) -> &AdtFields {
        let Value::Adt { fields, .. } = value else {
            panic!("expected an owned record");
        };
        fields
    }

    #[test]
    fn nested_branch_writes_copy_changed_paths_and_keep_other_fields_shared() {
        let context = Context::default();
        let original = record(vec![
            (
                "left".into(),
                record(vec![("count".into(), integer(&context, 7, 16, false))]),
            ),
            (
                "right".into(),
                record(vec![("count".into(), integer(&context, 11, 16, false))]),
            ),
        ]);
        let mut branch = original.clone();
        assert!(Rc::ptr_eq(&fields(&original).0, &fields(&branch).0));
        let Value::Adt { fields: outer, .. } = &mut branch else {
            panic!("expected a branch record");
        };
        let Value::Adt { fields: inner, .. } = &mut outer[0].1 else {
            panic!("expected a nested branch record");
        };
        inner[0].1 = integer(&context, 19, 16, false);
        assert!(!Rc::ptr_eq(&fields(&original).0, &fields(&branch).0));
        assert!(!Rc::ptr_eq(
            &fields(&fields(&original)[0].1).0,
            &fields(&fields(&branch)[0].1).0,
        ));
        assert!(Rc::ptr_eq(
            &fields(&fields(&original)[1].1).0,
            &fields(&fields(&branch)[1].1).0,
        ));
        assert_eq!(
            original
                .field("left")
                .unwrap()
                .field("count")
                .unwrap()
                .integer()
                .unwrap()
                .0,
            integer(&context, 7, 16, false).integer().unwrap().0,
        );
        assert_eq!(
            branch
                .field("left")
                .unwrap()
                .field("count")
                .unwrap()
                .integer()
                .unwrap()
                .0,
            integer(&context, 19, 16, false).integer().unwrap().0,
        );
    }

    #[test]
    fn consuming_shared_fields_preserves_other_owners_and_allocation_handles() {
        let original: AdtFields = vec![(
            "borrow".into(),
            Value::Reference {
                allocation: 23,
                projection: Vec::new(),
                mutable: true,
            },
        )]
        .into();
        let mut consumed: Vec<_> = original.clone().into_iter().collect();
        assert!(matches!(
            consumed[0].1,
            Value::Reference { allocation: 23, .. }
        ));
        consumed[0].1 = Value::Unit;
        assert!(matches!(
            original[0].1,
            Value::Reference { allocation: 23, .. }
        ));
    }

    #[test]
    fn mutating_iteration_detaches_shared_fields() {
        let original: AdtFields = vec![("pending".into(), Value::Uninitialized)].into();
        let mut changed = original.clone();
        for (_, field) in &mut changed {
            *field = Value::Unit;
        }
        assert!(matches!(original[0].1, Value::Uninitialized));
        assert!(matches!(changed[0].1, Value::Unit));
    }
}
