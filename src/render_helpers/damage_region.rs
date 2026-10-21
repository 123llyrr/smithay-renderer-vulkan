//! Minimal rectangle disjoint damage coverage without external dependencies.
use smallvec::SmallVec;
use smithay::utils::{Rectangle, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    x: [i32; 2],
    y: [i32; 2],
}

impl Bounds {
    fn from_xywh([x, y, width, height]: [i32; 4]) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let end = |start, extent| (i64::from(start) + i64::from(extent)).min(i64::from(i32::MAX)) as i32;
        let bounds = Self {
            x: [x, end(x, width)],
            y: [y, end(y, height)],
        };
        (bounds.x[0] < bounds.x[1] && bounds.y[0] < bounds.y[1]).then_some(bounds)
    }

    fn to_xywh(self) -> [i32; 4] {
        [self.x[0], self.y[0], self.x[1] - self.x[0], self.y[1] - self.y[0]]
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let x = [self.x[0].max(other.x[0]), self.x[1].min(other.x[1])];
        let y = [self.y[0].max(other.y[0]), self.y[1].min(other.y[1])];
        (x[0] < x[1] && y[0] < y[1]).then_some(Self { x, y })
    }

    fn joined(self, other: Self) -> Option<Self> {
        let adjacent = |a: [i32; 2], b: [i32; 2]| a[1] == b[0] || b[1] == a[0];
        if self.x == other.x && adjacent(self.y, other.y) {
            Some(Self {
                x: self.x,
                y: [self.y[0].min(other.y[0]), self.y[1].max(other.y[1])],
            })
        } else if self.y == other.y && adjacent(self.x, other.x) {
            Some(Self {
                x: [self.x[0].min(other.x[0]), self.x[1].max(other.x[1])],
                y: self.y,
            })
        } else {
            None
        }
    }

    fn without(self, cut: Self, mut emit: impl FnMut(Self)) {
        let Some(overlap) = self.intersection(cut) else {
            emit(self);
            return;
        };
        if self.y[0] < overlap.y[0] {
            emit(Self { x: self.x, y: [self.y[0], overlap.y[0]] });
        }
        if overlap.y[1] < self.y[1] {
            emit(Self { x: self.x, y: [overlap.y[1], self.y[1]] });
        }
        if self.x[0] < overlap.x[0] {
            emit(Self { x: [self.x[0], overlap.x[0]], y: overlap.y });
        }
        if overlap.x[1] < self.x[1] {
            emit(Self { x: [overlap.x[1], self.x[1]], y: overlap.y });
        }
    }
}

pub type DamageRegions<K> = SmallVec<[Rectangle<i32, K>; 8]>;

pub fn disjoint_damage<K>(
    damage: &[Rectangle<i32, K>],
    size: Size<i32, K>,
) -> DamageRegions<K> {
    match damage {
        [] => return DamageRegions::new(),
        [rect] => {
            let Some(viewport) = Bounds::from_xywh([0, 0, size.w, size.h]) else {
                return DamageRegions::new();
            };
            let Some(bounds) = Bounds::from_xywh([rect.loc.x, rect.loc.y, rect.size.w, rect.size.h])
                .and_then(|b| b.intersection(viewport)) else {
                return DamageRegions::new();
            };
            let [x, y, w, h] = bounds.to_xywh();
            return smallvec::smallvec![Rectangle::new((x, y).into(), (w, h).into())];
        }
        _ => {}
    }
    let Some(viewport) = Bounds::from_xywh([0, 0, size.w, size.h]) else {
        return DamageRegions::new();
    };
    let mut covered = SmallVec::<[Bounds; 8]>::new();
    let mut pending = SmallVec::<[(Bounds, usize); 8]>::new();
    let mut ready = SmallVec::<[Bounds; 8]>::new();

    for &input in damage {
        let Some(rect) = Bounds::from_xywh([input.loc.x, input.loc.y, input.size.w, input.size.h])
            .and_then(|b| b.intersection(viewport))
        else {
            continue;
        };
        if rect == viewport {
            let [x, y, w, h] = viewport.to_xywh();
            return smallvec::smallvec![Rectangle::new((x, y).into(), (w, h).into())];
        }
        let Some(first) = covered.iter().position(|prev| rect.intersection(*prev).is_some()) else {
            covered.push(rect);
            continue;
        };
        pending.push((rect, first));
        while let Some((candidate, check_idx)) = pending.pop() {
            if check_idx >= covered.len() {
                ready.push(candidate);
                continue;
            }
            let other = covered[check_idx];
            if candidate.intersection(other).is_none() {
                pending.push((candidate, check_idx + 1));
            } else {
                candidate.without(other, |fragment| {
                    pending.push((fragment, check_idx + 1));
                });
            }
        }
        while let Some(addition) = ready.pop() {
            let mut add = addition;
            if covered.len() <= 8 {
                let mut idx = 0;
                while idx < covered.len() {
                    if let Some(joined) = add.joined(covered[idx]) {
                        add = joined;
                        covered.swap_remove(idx);
                        idx = 0;
                    } else {
                        idx += 1;
                    }
                }
            }
            covered.push(add);
        }
    }
    covered
        .into_iter()
        .map(|b| {
            let [x, y, w, h] = b.to_xywh();
            Rectangle::new((x, y).into(), (w, h).into())
        })
        .collect()
}
