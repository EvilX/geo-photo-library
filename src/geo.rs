//! Web Mercator и кластеризация маркеров в пиксельных координатах.
use std::collections::HashMap;
use std::f64::consts::PI;

pub const TILE: f64 = 256.0;
const MAX_LAT: f64 = 85.051_128_78;

fn world(zoom: f64) -> f64 {
    TILE * zoom.exp2()
}

/// (lat, lon) → мировые пиксели на уровне `zoom`.
pub fn project(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let s = world(zoom);
    let sin = lat.clamp(-MAX_LAT, MAX_LAT).to_radians().sin();
    let x = (lon + 180.0) / 360.0 * s;
    let y = (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * PI)) * s;
    (x, y)
}

pub fn unproject(x: f64, y: f64, zoom: f64) -> (f64, f64) {
    let s = world(zoom);
    let lon = x / s * 360.0 - 180.0;
    let n = PI - 2.0 * PI * y / s;
    let lat = n.sinh().atan().to_degrees();
    (lat, lon)
}

/// Видимая область карты.
#[derive(Debug, Clone, Copy)]
pub struct View {
    cx: f64,
    cy: f64,
    /// 2^zoom: множитель от координат на зуме 0 к пикселям этого зума.
    scale: f64,
    half_w: f64,
    half_h: f64,
}

impl View {
    pub fn new(lat: f64, lon: f64, zoom: f64, width: f64, height: f64) -> Self {
        let (cx, cy) = project(lat, lon, zoom);
        Self { cx, cy, scale: zoom.exp2(), half_w: width / 2.0, half_h: height / 2.0 }
    }

    /// Попадает ли точка в область, расширенную на `margin` пикселей.
    #[cfg(test)]
    pub fn contains(&self, lat: f64, lon: f64, margin: f64) -> bool {
        let (x, y) = project(lat, lon, 0.0);
        self.contains_world(x, y, margin)
    }

    /// То же для точки, уже спроецированной на зум 0 ([`project`] с `zoom = 0`):
    /// без тригонометрии — для проверки десятков тысяч фото на каждое движение карты.
    pub fn contains_world(&self, x0: f64, y0: f64, margin: f64) -> bool {
        let s = self.scale;
        (x0 * s - self.cx).abs() <= self.half_w + margin && (y0 * s - self.cy).abs() <= self.half_h + margin
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub lat: f64,
    pub lon: f64,
    /// Индексы во входном массиве точек, по возрастанию.
    pub members: Vec<usize>,
}

/// Группирует точки по сетке `cell`×`cell` пикселей, привязанной к миру
/// (а не к экрану), поэтому при панорамировании кластеры не «прыгают».
#[cfg(test)]
pub fn cluster(points: &[(f64, f64)], zoom: f64, cell: f64) -> Vec<Cluster> {
    let world: Vec<(f64, f64)> = points.iter().map(|&(lat, lon)| project(lat, lon, 0.0)).collect();
    cluster_world(&world, zoom, cell)
}

/// [`cluster`] для точек, уже спроецированных на зум 0.
pub fn cluster_world(points: &[(f64, f64)], zoom: f64, cell: f64) -> Vec<Cluster> {
    let scale = zoom.exp2();
    let mut cells: HashMap<(i64, i64), usize> = HashMap::new();
    let mut acc: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for (i, &(x0, y0)) in points.iter().enumerate() {
        let (x, y) = (x0 * scale, y0 * scale);
        let key = ((x / cell).floor() as i64, (y / cell).floor() as i64);
        let slot = *cells.entry(key).or_insert_with(|| {
            acc.push((0.0, 0.0, Vec::new()));
            acc.len() - 1
        });
        let a = &mut acc[slot];
        a.0 += x;
        a.1 += y;
        a.2.push(i);
    }
    acc.into_iter()
        .map(|(sx, sy, members)| {
            let n = members.len() as f64;
            let (lat, lon) = unproject(sx / n, sy / n, zoom);
            Cluster { lat, lon, members }
        })
        .collect()
}

/// Уровень зума, при котором прямоугольник точек помещается в окно `w`×`h`
/// (с отступом `pad`). Для вырожденного прямоугольника — `f64::INFINITY`.
pub fn fit_zoom(points: &[(f64, f64)], w: f64, h: f64, pad: f64) -> f64 {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(lat, lon) in points {
        let (x, y) = project(lat, lon, 0.0);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (dx, dy) = (x1 - x0, y1 - y0);
    if points.is_empty() || (dx < 1e-9 && dy < 1e-9) {
        return f64::INFINITY;
    }
    let (w, h) = ((w - 2.0 * pad).max(1.0), (h - 2.0 * pad).max(1.0));
    (w / dx.max(1e-9)).min(h / dy.max(1e-9)).log2()
}

/// Центр прямоугольника точек (в проекции Меркатора).
pub fn center(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(lat, lon) in points {
        let (x, y) = project(lat, lon, 0.0);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Some(unproject((x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for &(lat, lon) in &[(55.75, 37.62), (-33.86, 151.2), (0.0, 0.0), (64.1, -21.9)] {
            let (x, y) = project(lat, lon, 12.3);
            let (a, b) = unproject(x, y, 12.3);
            assert!((a - lat).abs() < 1e-9 && (b - lon).abs() < 1e-9);
        }
        assert_eq!(project(0.0, 0.0, 0.0), (128.0, 128.0));
    }

    #[test]
    fn view_contains() {
        let v = View::new(55.75, 37.62, 10.0, 800.0, 600.0);
        assert!(v.contains(55.75, 37.62, 0.0));
        assert!(!v.contains(59.93, 30.31, 0.0)); // Петербург не виден на зуме 10 над Москвой
        let v = View::new(55.75, 37.62, 4.0, 800.0, 600.0);
        assert!(v.contains(59.93, 30.31, 0.0));
    }

    #[test]
    fn clustering() {
        let pts = [(55.7500, 37.6200), (55.7501, 37.6201), (59.93, 30.31)];
        let c = cluster(&pts, 5.0, 64.0);
        assert_eq!(c.len(), 2);
        assert!(c.iter().any(|c| c.members == [0, 1]));
        let c = cluster(&pts, 22.0, 64.0);
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn fit() {
        let pts = [(55.75, 37.62), (59.93, 30.31)];
        let z = fit_zoom(&pts, 800.0, 600.0, 0.0);
        assert!(z > 4.0 && z < 8.0, "{z}");
        assert!(fit_zoom(&pts[..1], 800.0, 600.0, 0.0).is_infinite());
        let (lat, lon) = center(&pts).unwrap();
        assert!(lat > 55.75 && lat < 59.93 && lon > 30.31 && lon < 37.62);
    }
}
