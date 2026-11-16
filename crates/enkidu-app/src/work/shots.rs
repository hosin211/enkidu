//! A report's screenshots (DESIGN.md, "Claude's report"): thumbnails in the card, and one as large as the window, ←
//! and → through the report's others, Esc back to the card.
//!
//! A picture is read and shrunk off the drawing thread, one after another (a report can hold twelve, and each is a
//! whole picture in memory while it is shrunk). A thumbnail keeps only what it shows: 16 to 10 from the picture's
//! top, 920 px wide at most. The large one is read again at the screen's size and kept only while it shows. What was
//! drawn is taken out of the window's atlas again when it goes, or every picture ever shown would stay in it.
//!
//! A recording (a GIF) is known small by its first picture that shows something. Large, it plays at its own size
//! from its file, a picture at a time: read whole, a recording of forty seconds is close to a gigabyte of pixels.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use enkidu_model::work::report::{
    THUMB_WIDTH, THUMB_WIDTH_LARGE, THUMB_WIDTH_LARGE_OF_MANY, THUMB_WIDTH_OF_MANY, fit_within, shot_name, step_shot,
    thumb_of,
};
use enkidu_ui::tokens::{self as theme, alpha, bezier};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AsyncApp, Context, FocusHandle, FontWeight, Global, IntoElement,
    ObjectFit, Render, RenderImage, Role, SharedString, Task, WeakEntity, Window, actions, anchored, deferred, div,
    img, point, px, rgb, svg,
};
use image::{DynamicImage, ImageDecoder as _, ImageReader, RgbaImage};

use crate::bridge::Bridge;

/// The large screenshot's own keys work while it is on screen, and the card's don't.
pub const KEYS: &str = "WorkShot";

actions!(work_shot, [NextShot, PrevShot, CloseShot]);

/// Reads one screenshot of a report, by the report's id and the picture's name: its bytes, or why not.
pub type Read = Arc<dyn Fn(i64, &str) -> Result<Vec<u8>, String> + Send + Sync>;

/// Where screenshots are read in a window without the core (a screen test, a picture). The app reads the core's.
pub struct ShotSource(pub Read);

impl Global for ShotSource {}

fn reader(cx: &App) -> Option<Read> {
    if let Some(source) = cx.try_global::<ShotSource>() {
        return Some(source.0.clone());
    }
    if !cx.has_global::<Bridge>() {
        return None;
    }
    let (shared, _) = Bridge::core(cx);
    Some(Arc::new(move |report, name| {
        enkidu_core::commands::work_report_image(&shared, report, name.to_string()).map_err(|err| err.message)
    }))
}

/// A picture that would need more memory than this while it is read isn't shown.
const MOST_BYTES: u64 = 256 * 1024 * 1024;

/// The picture in `bytes`, whatever its name says it is, turned as its own data says.
fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let said = |err: image::ImageError| err.to_string();
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|err| err.to_string())?;
    let mut decoder = reader.into_decoder().map_err(said)?;
    if decoder.total_bytes() > MOST_BYTES {
        return Err("The picture is too large to show.".to_string());
    }
    let turned = decoder.orientation().map_err(said)?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(said)?;
    picture.apply_orientation(turned);
    Ok(picture)
}

/// The window's atlas wants blue first.
fn for_the_window(mut pixels: RgbaImage) -> RgbaImage {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    pixels
}

fn drawn(pixels: RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// What kind of file a picture is, by its own first bytes (whatever its name says): PNG, JPG, GIF or WEBP, as a
/// thumbnail's label says it. Nothing for any other kind.
pub fn kind_of(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => "PNG",
        Ok(image::ImageFormat::Jpeg) => "JPG",
        Ok(image::ImageFormat::Gif) => "GIF",
        Ok(image::ImageFormat::WebP) => "WEBP",
        _ => "",
    }
}

/// What a thumbnail keeps of the picture in `bytes`, never wider than `most`: the part it shows, the whole picture's
/// size, and whether the part is all of it.
pub fn small_of(bytes: &[u8], most: u32) -> Result<(RgbaImage, (u32, u32), bool), String> {
    small_from(still(bytes)?, most)
}

/// [`small_of`] for a picture that is read already (one picture of a recording, too).
fn small_from(picture: DynamicImage, most: u32) -> Result<(RgbaImage, (u32, u32), bool), String> {
    let whole = (picture.width(), picture.height());
    let thumb = thumb_of(whole.0, whole.1, most).ok_or("The picture is empty.")?;
    let (left, top, width, height) = thumb.part;
    let covers = thumb.part == (0, 0, whole.0, whole.1);
    let part = if covers { picture } else { picture.crop_imm(left, top, width, height) };
    let kept = match thumb.size == (width, height) {
        true => part.into_rgba8(),
        false => part.thumbnail_exact(thumb.size.0, thumb.size.1).into_rgba8(),
    };
    Ok((for_the_window(kept), whole, covers))
}

/// The picture in `bytes`, whole, never larger than `room` (the screen's own pixels).
pub fn large_of(bytes: &[u8], room: (u32, u32)) -> Result<RgbaImage, String> {
    let picture = still(bytes)?;
    let size = fit_within(picture.width(), picture.height(), room);
    let kept = match size == (picture.width(), picture.height()) {
        true => picture.into_rgba8(),
        false => picture.thumbnail_exact(size.0, size.1).into_rgba8(),
    };
    Ok(for_the_window(kept))
}

/// The frames of a picture that moves (a GIF, an animated WebP), read one after another from `bytes`; none for a
/// picture that can't move.
fn moving_frames(bytes: &[u8]) -> Result<Option<image::Frames<'_>>, String> {
    use image::AnimationDecoder as _;
    let said = |err: image::ImageError| err.to_string();
    let too_large = |bytes: u64| match bytes > MOST_BYTES {
        true => Err("The picture is too large to show.".to_string()),
        false => Ok(()),
    };
    Ok(match image::guess_format(bytes).map_err(said)? {
        image::ImageFormat::Gif => {
            let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(said)?;
            too_large(decoder.total_bytes())?;
            Some(decoder.into_frames())
        }
        image::ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(said)?;
            too_large(decoder.total_bytes())?;
            decoder.has_animation().then(|| decoder.into_frames())
        }
        _ => None,
    })
}

/// Whether a picture shows nothing: every pixel the same.
fn blank(picture: &RgbaImage) -> bool {
    let mut pixels = picture.pixels();
    pixels.next().is_none_or(|first| pixels.all(|pixel| pixel == first))
}

/// How many pictures of a recording are looked at for the one it is known by.
const LOOKED_AT: usize = 80;

/// The picture a recording is known by: its first one that shows something. Many recordings start on an empty page,
/// and a thumbnail of that is a white box. None for a picture that doesn't move (one frame alone is a still picture).
pub fn telling_frame(bytes: &[u8]) -> Option<RgbaImage> {
    let mut frames = moving_frames(bytes).ok()??;
    let first = frames.next()?.ok()?.into_buffer();
    let second = frames.next()?.ok()?.into_buffer();
    if !blank(&first) {
        return Some(first);
    }
    if !blank(&second) {
        return Some(second);
    }
    for frame in frames.take(LOOKED_AT) {
        let Ok(frame) = frame else { break };
        let picture = frame.into_buffer();
        if !blank(&picture) {
            return Some(picture);
        }
    }
    Some(first)
}

/// The picture in `bytes` as it stands still: a recording's first picture that shows something, any other picture
/// as it is.
fn still(bytes: &[u8]) -> Result<DynamicImage, String> {
    match telling_frame(bytes) {
        Some(picture) => Ok(DynamicImage::ImageRgba8(picture)),
        None => decode(bytes),
    }
}

/// One picture of a recording as it plays: its pixels as the window wants them, and how long it stays.
pub type Step = (RgbaImage, std::time::Duration);

/// How many pictures of a recording are read ahead of the one on screen.
pub const AHEAD: usize = 2;

/// A picture of a recording that says it stays no time at all stays this long, as browsers play it.
const SHORTEST_STEP: std::time::Duration = std::time::Duration::from_millis(20);
const STEP_OF_NONE: std::time::Duration = std::time::Duration::from_millis(100);

/// Plays the recording in `bytes` on a thread of its own, around and around: each picture is there when it is taken,
/// at its own size (never larger than `room`), and the thread ends when nobody takes them any more. Only [`AHEAD`]
/// pictures wait to be taken, so a recording of any length takes the memory of a few of its pictures: it is read
/// from its file as it plays, never all of it at once. Nothing comes for a picture that doesn't move.
pub fn play(bytes: Arc<Vec<u8>>, room: (u32, u32)) -> std::sync::mpsc::Receiver<Step> {
    play_counting(bytes, room, Arc::new(std::sync::atomic::AtomicUsize::new(0)))
}

/// [`play`] for a thumbnail: each picture cut and shrunk as [`small_of`] does it, never wider than `most`.
pub fn play_small(bytes: Arc<Vec<u8>>, most: u32) -> std::sync::mpsc::Receiver<Step> {
    let cut = move |whole: RgbaImage| small_from(DynamicImage::ImageRgba8(whole), most).ok().map(|small| small.0);
    play_cut(bytes, Box::new(cut), Arc::new(std::sync::atomic::AtomicUsize::new(0)))
}

/// [`play`], counting in `read` each picture as it is read (a test sees how far ahead the reading goes).
fn play_counting(
    bytes: Arc<Vec<u8>>,
    room: (u32, u32),
    read: Arc<std::sync::atomic::AtomicUsize>,
) -> std::sync::mpsc::Receiver<Step> {
    let cut = move |whole: RgbaImage| {
        let size = fit_within(whole.width(), whole.height(), room);
        Some(for_the_window(match size == whole.dimensions() {
            true => whole,
            false => image::imageops::thumbnail(&whole, size.0, size.1),
        }))
    };
    play_cut(bytes, Box::new(cut), read)
}

/// What a recording's picture is made into before it is shown (nothing: the recording ends there).
type Cut = Box<dyn Fn(RgbaImage) -> Option<RgbaImage> + Send>;

/// The reading itself: each picture through `cut`, [`AHEAD`] of them waiting at most.
fn play_cut(
    bytes: Arc<Vec<u8>>,
    cut: Cut,
    read: Arc<std::sync::atomic::AtomicUsize>,
) -> std::sync::mpsc::Receiver<Step> {
    let (steps, taken) = std::sync::mpsc::sync_channel::<Step>(AHEAD);
    let playing = std::thread::Builder::new().name("recording".into()).spawn(move || {
        loop {
            let Ok(Some(frames)) = moving_frames(&bytes) else { return };
            let mut sent = 0usize;
            for frame in frames {
                // a recording that breaks off plays as far as it can be read
                let Ok(frame) = frame else { break };
                let stays = std::time::Duration::from(frame.delay());
                let stays = if stays < SHORTEST_STEP { STEP_OF_NONE } else { stays };
                let Some(picture) = cut(frame.into_buffer()) else { break };
                read.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if steps.send((picture, stays)).is_err() {
                    // nobody takes them any more: the view closed, or went on to another picture
                    return;
                }
                sent += 1;
            }
            if sent < 2 {
                return;
            }
        }
    });
    // (a thread that can't be started plays nothing: the picture is then said to be one that can't be shown)
    drop(playing);
    taken
}

/// One screenshot, small.
pub enum Small {
    Reading,
    Ready {
        /// What the thumbnail shows: for a recording that plays, the one of its pictures on screen now.
        picture: Arc<RenderImage>,
        /// The whole picture's size, in its own pixels.
        whole: (u32, u32),
        /// The thumbnail shows all of the picture, not a part.
        covers: bool,
        /// What kind of file it is, as its label says: PNG, JPG, GIF, WEBP (nothing for a kind without a name).
        kind: &'static str,
        /// It is a recording, and it plays by itself here.
        plays: bool,
        /// Whether it is in sight, as its own painting tells.
        sight: Arc<Sight>,
    },
    Failed,
}

/// Whether a thumbnail is in sight, told by its own painting: how many times it was painted, and whether the last
/// time it stood inside what the window shows. A recording goes on only after a paint that saw it.
#[derive(Default)]
pub struct Sight {
    painted: std::sync::atomic::AtomicU64,
    seen: std::sync::atomic::AtomicBool,
}

impl Sight {
    /// The thumbnail is painted now, in sight (`seen`) or outside what the window shows.
    pub fn painted(&self, seen: bool) {
        self.seen.store(seen, std::sync::atomic::Ordering::SeqCst);
        self.painted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// How many times it was painted.
    pub fn count(&self) -> u64 {
        self.painted.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether its last paint saw it.
    pub fn seen(&self) -> bool {
        self.seen.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Whether a recording that plays in its thumbnail may go on to its next picture: a paint saw the thumbnail since
/// the picture on screen was put there (`shown_at` is how often it had been painted then), and no screenshot is shown
/// large over it. So nothing goes on, and nothing is drawn, for a thumbnail that is scrolled away, covered, or in a
/// window that isn't drawn.
fn may_go_on(covered: bool, sight: &Sight, shown_at: u64) -> bool {
    !covered && sight.seen() && sight.count() > shown_at
}

/// How long a playing thumbnail waits between two looks for a paint that saw it: the first ten, then the others.
const QUICK_LOOK: std::time::Duration = std::time::Duration::from_millis(30);
const SLOW_LOOK: std::time::Duration = std::time::Duration::from_millis(300);

/// The screenshot shown as large as the window.
struct Large {
    report: i64,
    names: Vec<String>,
    at: usize,
    /// It, at the screen's size, once it is read.
    picture: Option<Arc<RenderImage>>,
    /// It is a recording, and it plays: the picture is the one of its pictures on screen now.
    playing: bool,
    failed: bool,
    /// The reading of it: gone when another one is asked for.
    reading: Option<Task<()>>,
}

/// The screenshots of the reports on screen.
pub struct Shots {
    /// The thumbnails are drawn large (a whole page shows them), so they are kept larger.
    wide: bool,
    small: HashMap<i64, HashMap<String, Small>>,
    /// The readings under way: dropped with the store, what hasn't started never starts.
    reading: Vec<Task<()>>,
    /// The recordings that play in their thumbnails, by report and name: each ends when it is dropped.
    playing: HashMap<(i64, String), Task<()>>,
    large: Option<Large>,
    focus: FocusHandle,
    /// What had the keyboard before a screenshot was shown large: it gets it back.
    before: Option<FocusHandle>,
}

impl Shots {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // the window keeps every picture it drew until it is told to let go
        cx.on_release(|shots, cx| shots.let_go(None, cx)).detach();
        Shots {
            wide: false,
            small: HashMap::new(),
            reading: Vec::new(),
            playing: HashMap::new(),
            large: None,
            focus: cx.focus_handle(),
            before: None,
        }
    }

    /// The same for a whole page, where the thumbnails are drawn large.
    pub fn wide(cx: &mut Context<Self>) -> Self {
        Shots { wide: true, ..Shots::new(cx) }
    }

    /// A report's screenshots are on screen: the ones not asked for yet are read, one after another.
    pub fn want(&mut self, report: i64, names: &[String], cx: &mut Context<Self>) {
        let known = self.small.entry(report).or_default();
        if names.iter().all(|name| known.contains_key(name)) {
            return;
        }
        let new: Vec<String> = names.iter().filter(|name| !known.contains_key(*name)).cloned().collect();
        let Some(read) = reader(cx) else {
            known.extend(new.into_iter().map(|name| (name, Small::Failed)));
            return;
        };
        known.extend(new.iter().map(|name| (name.clone(), Small::Reading)));
        // one alone is drawn larger than one of several, and on a whole page both are drawn large
        let most = match (self.wide, names.len() == 1) {
            (false, true) => THUMB_WIDTH,
            (false, false) => THUMB_WIDTH_OF_MANY,
            (true, true) => THUMB_WIDTH_LARGE,
            (true, false) => THUMB_WIDTH_LARGE_OF_MANY,
        };
        let stands_still = cx.reduce_motion();
        let task = cx.spawn(async move |shots: WeakEntity<Self>, cx: &mut AsyncApp| {
            for name in new {
                let (read, wanted) = (read.clone(), name.clone());
                let made = cx
                    .background_spawn(async move {
                        let bytes = read(report, &wanted)?;
                        // (a recording is known by its first picture that shows something)
                        let telling = telling_frame(&bytes);
                        let moves = telling.is_some();
                        let picture = match telling {
                            Some(picture) => DynamicImage::ImageRgba8(picture),
                            None => decode(&bytes)?,
                        };
                        small_from(picture, most).map(|small| (small, kind_of(&bytes), moves))
                    })
                    .await;
                let kept = shots.update(cx, |shots, cx| {
                    // forgotten meanwhile (another card took the dialog): nothing is kept
                    if shots.small.get(&report).is_none_or(|known| !known.contains_key(&name)) {
                        return;
                    }
                    let small = match made {
                        Ok(((pixels, whole, covers), kind, moves)) => {
                            let sight = Arc::new(Sight::default());
                            // a recording plays by itself, where things may move
                            let plays = moves && !stands_still;
                            if plays {
                                let player = shots.player(report, name.clone(), most, sight.clone(), cx);
                                shots.playing.insert((report, name.clone()), player);
                            }
                            Small::Ready { picture: drawn(pixels), whole, covers, kind, plays, sight }
                        }
                        Err(_) => Small::Failed,
                    };
                    if let Some(slot) = shots.small.get_mut(&report).and_then(|known| known.get_mut(&name)) {
                        *slot = small;
                    }
                    cx.notify();
                });
                if kept.is_err() {
                    return;
                }
            }
        });
        self.reading.push(task);
    }

    /// Plays a recording in its thumbnail, around and around. The next picture goes on screen only after a paint
    /// that saw the thumbnail ([`Sight`]), so one that is scrolled away, behind a screenshot shown large, or in a
    /// window that isn't drawn waits, and no frame is drawn for it.
    fn player(&self, report: i64, name: String, most: u32, sight: Arc<Sight>, cx: &mut Context<Self>) -> Task<()> {
        let Some(read) = reader(cx) else { return Task::ready(()) };
        cx.spawn(async move |shots: WeakEntity<Self>, cx: &mut AsyncApp| {
            let wanted = name.clone();
            let Ok(bytes) = cx.background_spawn(async move { read(report, &wanted) }).await else { return };
            let mut steps = play_small(Arc::new(bytes), most);
            // how often the thumbnail had been painted when the picture now on screen was put there
            let mut shown_at = 0u64;
            loop {
                // a paint that saw it since then, and no screenshot shown large over it: a few quick looks (the
                // next frame is on its way), then a slow watch that costs nothing to speak of
                let mut looks = 0u32;
                loop {
                    let Ok(covered) = shots.read_with(cx, |shots, _| shots.large.is_some()) else { return };
                    if may_go_on(covered, &sight, shown_at) {
                        break;
                    }
                    looks += 1;
                    let wait = if looks <= 10 { QUICK_LOOK } else { SLOW_LOOK };
                    cx.background_executor().timer(wait).await;
                }
                // (taken off the drawing thread: the next picture may still be on its way)
                let next = cx.background_spawn(async move { steps.recv().ok().map(|step| (step, steps)) }).await;
                let Some(((pixels, stays), back)) = next else { return };
                steps = back;
                let on = shots
                    .update(cx, |shots, cx| {
                        let slot = shots.small.get_mut(&report).and_then(|known| known.get_mut(&name));
                        let Some(Small::Ready { picture, .. }) = slot else { return false };
                        // (no window is being updated here, so the one before it leaves every window's atlas)
                        let previous = std::mem::replace(picture, drawn(pixels));
                        cx.drop_image(previous, None);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !on {
                    return;
                }
                shown_at = sight.count();
                cx.background_executor().timer(stays).await;
            }
        })
    }

    /// How far a screenshot is: none when it was never asked for.
    pub fn small(&self, report: i64, name: &str) -> Option<&Small> {
        self.small.get(&report)?.get(name)
    }

    /// The screenshot shown large now: its report and its name.
    pub fn shown(&self) -> Option<(i64, &str)> {
        let large = self.large.as_ref()?;
        Some((large.report, large.names.get(large.at)?.as_str()))
    }

    /// Shows the screenshot at `at` of a report's `names` as large as the window; the keyboard is its own until it
    /// closes.
    pub fn open(&mut self, report: i64, names: Vec<String>, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if at >= names.len() {
            return;
        }
        self.close_large(window, cx);
        self.before = window.focused(cx);
        self.large = Some(Large { report, names, at, picture: None, playing: false, failed: false, reading: None });
        self.read_large(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// → and ←: the report's next screenshot, or the one before, around the ends. One alone stays.
    pub fn step(&mut self, by: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(large) = &mut self.large else { return };
        let next = step_shot(large.names.len(), large.at, by);
        if next == large.at {
            return;
        }
        large.at = next;
        self.read_large(window, cx);
        cx.notify();
    }

    /// The large screenshot goes; the keyboard returns to where it was.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.large.is_none() {
            return;
        }
        self.close_large(window, cx);
        if let Some(before) = self.before.take() {
            before.focus(window, cx);
        }
        cx.notify();
    }

    /// Whether a screenshot is shown large.
    pub fn is_open(&self) -> bool {
        self.large.is_some()
    }

    /// The whole picture of the screenshot shown large, once it is read.
    pub fn large_picture(&self) -> Option<&Arc<RenderImage>> {
        self.large.as_ref()?.picture.as_ref()
    }

    /// Another card took the dialog: this one's pictures go.
    pub fn forget(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.let_go(Some(window), cx);
        cx.notify();
    }

    fn close_large(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(picture) = self.large.take().and_then(|large| large.picture) {
            cx.drop_image(picture, Some(window));
        }
    }

    /// Everything read is dropped, and taken out of the window's atlas. `window` is the one being updated, if one is
    /// (it stands outside the app's list meanwhile).
    fn let_go(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        self.reading.clear();
        self.playing.clear();
        let small = std::mem::take(&mut self.small);
        let large = self.large.take().and_then(|large| large.picture);
        let pictures = small.into_values().flat_map(HashMap::into_values).filter_map(|small| match small {
            Small::Ready { picture, .. } => Some(picture),
            _ => None,
        });
        for picture in pictures.chain(large) {
            cx.drop_image(picture, window.as_deref_mut());
        }
    }

    /// Reads the screenshot on screen at the screen's size: the window may grow while it shows, never past that. A
    /// recording plays from its file, a picture at a time (see [`play`]); with "Reduce motion" it is the one picture
    /// it is known by.
    fn read_large(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(large) = &mut self.large else { return };
        if let Some(previous) = large.picture.take() {
            cx.drop_image(previous, Some(window));
        }
        large.failed = false;
        large.playing = false;
        let Some(read) = reader(cx) else {
            large.failed = true;
            large.reading = None;
            return;
        };
        let (report, at, name) = (large.report, large.at, large.names[large.at].clone());
        let (view, scale) = (window.viewport_size(), window.scale_factor());
        let screen = window.display(cx).map(|display| display.bounds().size).unwrap_or(view);
        let pixels = |points: gpui_kit::Pixels| (f32::from(points) * scale).ceil().max(1.) as u32;
        let room = (pixels(screen.width.max(view.width)), pixels(screen.height.max(view.height)));
        let stands_still = cx.reduce_motion();
        large.reading = Some(cx.spawn(async move |shots: WeakEntity<Self>, cx: &mut AsyncApp| {
            // the picture goes on screen, in place of the one before it; false when you went on to another one
            // meanwhile, or closed it
            let show = |shots: &WeakEntity<Self>, cx: &mut AsyncApp, pixels: RgbaImage, playing: bool| {
                shots
                    .update(cx, |shots, cx| {
                        let Some(large) = shots.large.as_mut().filter(|large| large.report == report && large.at == at)
                        else {
                            return false;
                        };
                        // (no window is being updated here, so the one before it leaves every window's atlas)
                        if let Some(previous) = large.picture.replace(drawn(pixels)) {
                            cx.drop_image(previous, None);
                        }
                        large.playing = playing;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false)
            };
            let fail = |shots: &WeakEntity<Self>, cx: &mut AsyncApp| {
                let _ = shots.update(cx, |shots, cx| {
                    if let Some(large) = shots.large.as_mut().filter(|large| large.report == report && large.at == at) {
                        large.failed = true;
                        cx.notify();
                    }
                });
            };
            let made = cx
                .background_spawn(async move {
                    let bytes = Arc::new(read(report, &name)?);
                    match !stands_still && telling_frame(&bytes).is_some() {
                        true => Ok((Some(bytes), None)),
                        false => large_of(&bytes, room).map(|picture| (None, Some(picture))),
                    }
                })
                .await;
            match made {
                Err(_) => fail(&shots, cx),
                Ok((_, Some(picture))) => {
                    show(&shots, cx, picture, false);
                }
                Ok((Some(recording), None)) => {
                    let mut steps = play(recording, room);
                    let mut shown = false;
                    loop {
                        // (taken off the drawing thread: the next picture may still be on its way)
                        let next =
                            cx.background_spawn(async move { steps.recv().ok().map(|step| (step, steps)) }).await;
                        let Some(((picture, stays), back)) = next else { break };
                        steps = back;
                        if !show(&shots, cx, picture, true) {
                            return;
                        }
                        shown = true;
                        cx.background_executor().timer(stays).await;
                    }
                    if !shown {
                        fail(&shots, cx);
                    }
                }
                Ok((None, None)) => {}
            }
        }));
    }
}

/// The large screenshot's rank among what is drawn late: over a dialog's surface (which the library draws at a
/// popup's rank), under a tooltip (200). What is drawn late is painted by rank alone, wherever it stands in the tree:
/// at a lower rank the card would be painted over the screenshot it opened.
const OVER_THE_CARD: usize = 150;
const _: () = assert!(OVER_THE_CARD > gpui_kit::base::POPUP_PRIORITY);

/// The room around the large screenshot, and its top row's height.
const AROUND: f32 = 24.;
const TOP_ROW: f32 = 40.;

/// A round button over the picture's side: the screenshot before, or the next.
fn side_button(id: &'static str, label: &'static str, icon: &'static str, left: bool) -> enkidu_ui::pieces::Pressed {
    let dark = |strength: f32| alpha(rgb(0x000000), strength);
    enkidu_ui::press::pressable(div().id(id))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .absolute()
        .top_1_2()
        .mt(px(-18.))
        .when(left, |button| button.left_0())
        .when(!left, |button| button.right_0())
        .size(px(36.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(dark(0.55))
        .cursor_pointer()
        .hover(move |s| s.bg(dark(0.75)))
        .child(svg().path(icon).flex_none().size(px(18.)).text_color(rgb(0xffffff)))
}

impl Render for Shots {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(large) = &self.large else {
            return div().into_any_element();
        };
        let (count, at) = (large.names.len(), large.at);
        let name = large.names[at].clone();
        let small = self.small(large.report, &name);
        let whole = match small {
            Some(Small::Ready { whole, .. }) => Some(*whole),
            _ => None,
        };
        // the picture itself once it is read; until then its thumbnail, when that shows all of it
        let stand_in = match small {
            Some(Small::Ready { picture, covers: true, .. }) => Some(picture.clone()),
            _ => None,
        };
        let picture = large.picture.clone().or(stand_in);
        let failed = large.failed || matches!(small, Some(Small::Failed));
        let view = window.viewport_size();
        // as large as the window allows, never larger than the picture is
        let room = (f32::from(view.width) - 2. * AROUND, f32::from(view.height) - 2. * AROUND - TOP_ROW);
        let size = whole.map(|(width, height)| {
            let scale = (room.0 / width as f32).min(room.1 / height as f32).min(1.);
            (width as f32 * scale, height as f32 * scale)
        });
        let position = SharedString::from(format!("{} of {count}", at + 1));
        let white = |strength: f32| alpha(rgb(0xffffff), strength);
        let top = div()
            .flex_none()
            .h(px(TOP_ROW))
            .pb(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .text_size(px(12.5))
            .text_color(white(0.8))
            .child(div().min_w_0().truncate().child(SharedString::from(shot_name(&name).to_string())))
            .when(count > 1, |row| row.child(div().flex_none().text_color(white(0.5)).child(position.clone())))
            .child(div().flex_1())
            .child(
                enkidu_ui::press::pressable(div().id("report-lightbox-close"))
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Close")
                    .flex_none()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(white(0.1)))
                    .child(svg().path("icons/x.svg").flex_none().size(px(16.)).text_color(white(0.8)))
                    .on_click(cx.listener(|shots, _, window, cx| shots.close(window, cx))),
            );
        let shown: Option<AnyElement> = match (picture, size) {
            (Some(picture), Some((width, height))) => Some(
                // (a picture element says nothing of itself, so a box around it carries its name)
                div()
                    .id("report-lightbox-image")
                    .test_support()
                    .aria_label(SharedString::from(name.clone()))
                    // (a recording says that it plays)
                    .aria_description(match (large.picture.is_some(), large.playing) {
                        (false, _) => "its thumbnail meanwhile",
                        (true, false) => "whole",
                        (true, true) => "whole, playing",
                    })
                    .flex_none()
                    .w(px(width))
                    .h(px(height))
                    // a click on the picture is not a click beside it
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(img(picture).size_full().rounded(px(6.)).object_fit(ObjectFit::Fill))
                    .into_any_element(),
            ),
            // read, but its thumbnail shows a part only: its place until the whole one is there
            (None, Some((width, height))) if !failed => {
                Some(div().flex_none().w(px(width)).h(px(height)).rounded(px(6.)).bg(white(0.04)).into_any_element())
            }
            _ if failed => Some(
                div()
                    .id("report-lightbox-failed")
                    .test_support()
                    .aria_label(SharedString::from(format!("Can't show {name}")))
                    .text_size(px(12.5))
                    .text_color(white(0.6))
                    .child(SharedString::from(format!("Can't show {name}")))
                    .into_any_element(),
            ),
            _ => None,
        };
        let stage = div().relative().flex_1().min_h_0().flex().items_center().justify_center().children(shown).when(
            count > 1,
            |stage| {
                stage
                    .child(side_button("report-lightbox-prev", "Previous", "icons/chevron-left.svg", true).on_click(
                        cx.listener(|shots, _, window, cx| {
                            cx.stop_propagation();
                            shots.step(-1, window, cx);
                        }),
                    ))
                    .child(side_button("report-lightbox-next", "Next", "icons/chevron-right.svg", false).on_click(
                        cx.listener(|shots, _, window, cx| {
                            cx.stop_propagation();
                            shots.step(1, window, cx);
                        }),
                    ))
            },
        );
        let layer = div()
            .id("report-lightbox")
            .test_support()
            .aria_label(SharedString::from(name))
            .aria_description(position)
            .key_context(KEYS)
            .track_focus(&self.focus)
            .on_action(cx.listener(|shots, _: &NextShot, window, cx| shots.step(1, window, cx)))
            .on_action(cx.listener(|shots, _: &PrevShot, window, cx| shots.step(-1, window, cx)))
            // Esc closes the screenshot, not the card under it
            .on_action(cx.listener(|shots, _: &CloseShot, window, cx| shots.close(window, cx)))
            // nothing under it hears the mouse; a click beside the picture closes it
            .occlude()
            .on_click(cx.listener(|shots, _, window, cx| shots.close(window, cx)))
            .w(view.width)
            .h(view.height)
            .p(px(AROUND))
            .flex()
            .flex_col()
            .bg(alpha(rgb(0x000000), 0.93))
            .font_weight(FontWeight::NORMAL)
            .child(top)
            .child(stage)
            .with_animation(
                "report-lightbox-in",
                Animation::new(theme::DUR_QUICK).with_easing(|t| bezier(theme::EASE_OUT, t)),
                |layer, t| layer.opacity(t),
            );
        // over the card: a dialog draws its surface late, at a popup's rank (100), and a tooltip later still (200)
        deferred(anchored().position(point(px(0.), px(0.))).child(layer))
            .with_priority(OVER_THE_CARD)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A recording of pictures, each `width` by `height`, 100 ms apart: one for each shade, its left half that shade
    /// of red and its right half the same shade of blue. A shade of 255 is an empty page instead: all white.
    fn recording_of(shades: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut bytes, 30);
            for shade in shades {
                let picture = RgbaImage::from_fn(width, height, |x, _| match (*shade, x < width / 2) {
                    (255, _) => image::Rgba([255, 255, 255, 255]),
                    (shade, true) => image::Rgba([shade, 0, 0, 255]),
                    (shade, false) => image::Rgba([0, 0, shade, 255]),
                });
                let delay = image::Delay::from_numer_denom_ms(100, 1);
                encoder.encode_frame(image::Frame::from_parts(picture, 0, 0, delay)).unwrap();
            }
        }
        bytes
    }

    fn recording(frames: usize, width: u32, height: u32) -> Vec<u8> {
        let shades: Vec<u8> = (0..frames).map(|n| (40 + n * 50) as u8).collect();
        recording_of(&shades, width, height)
    }

    /// The red of a step's first pixel (blue comes first, as the window's atlas wants it: red is the third byte).
    fn red(pixels: &RgbaImage) -> u8 {
        pixels.get_pixel(0, 0).0[2]
    }

    /// Item 7.5b: a GIF in a report plays in the large view, one picture after another, at its own size, and around.
    #[test]
    fn a_recording_plays_picture_by_picture_at_its_own_size_and_around() {
        let steps = play(Arc::new(recording(4, 64, 40)), (2000, 2000));
        let first: Vec<Step> = (0..4).map(|_| steps.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
        assert!(first.iter().all(|(pixels, _)| pixels.dimensions() == (64, 40)), "each at the recording's own size");
        assert!(first.iter().all(|(_, stays)| *stays == Duration::from_millis(100)), "each for its own time");
        for ((pixels, _), want) in first.iter().zip([40u8, 90, 140, 190]) {
            assert!(red(pixels).abs_diff(want) <= 4, "in the recording's order: {} for {want}", red(pixels));
        }
        // after the last one it starts again
        let (again, _) = steps.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(red(&again).abs_diff(40) <= 4, "around: {}", red(&again));
        // never larger than the room it is shown in
        let small = play(Arc::new(recording(3, 64, 40)), (32, 32));
        assert_eq!(small.recv_timeout(Duration::from_secs(10)).unwrap().0.dimensions(), (32, 20));
        // a still picture doesn't play: a PNG, and a GIF of one picture
        let mut png = Vec::new();
        let still = RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        image::DynamicImage::ImageRgba8(still).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(play(Arc::new(png.clone()), (2000, 2000)).recv_timeout(Duration::from_secs(10)).is_err());
        assert!(telling_frame(&png).is_none() && telling_frame(&recording(1, 64, 40)).is_none());
    }

    /// A recording of any length takes the memory of a few of its pictures: only the next ones are read ahead, and
    /// the reading ends when nobody takes them any more. (Read whole, the 14 recordings of one real night's reports
    /// were 177 to 946 MB each; to stay within 64 MB they were played at a quarter of their width.)
    #[test]
    fn a_long_recording_is_read_a_few_pictures_ahead_and_no_further() {
        let bytes = Arc::new(recording(60, 64, 40));
        let read = Arc::new(AtomicUsize::new(0));
        let steps = play_counting(bytes.clone(), (2000, 2000), read.clone());
        // nothing is taken for a while: the reading waits
        std::thread::sleep(Duration::from_millis(300));
        let ahead = read.load(Ordering::SeqCst);
        assert!((1..=AHEAD + 1).contains(&ahead), "{ahead} of 60 pictures were read while none was taken");
        // each one taken lets one more be read
        for _ in 0..10 {
            steps.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        std::thread::sleep(Duration::from_millis(300));
        let now = read.load(Ordering::SeqCst);
        assert!((10..=10 + AHEAD + 1).contains(&now), "{now} read after 10 were taken");
        // the large view closes: the reading ends, and lets the recording go
        drop(steps);
        let waited = std::time::Instant::now();
        while Arc::strong_count(&bytes) > 1 {
            assert!(waited.elapsed() < Duration::from_secs(10), "the reading never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A recording is known by its first picture that shows something: many start on an empty page, and a thumbnail
    /// of that is a white box.
    #[test]
    fn a_recording_is_known_by_its_first_picture_that_shows_something() {
        // three empty pictures, then the page
        let starts_empty = recording_of(&[255, 255, 255, 90, 140], 64, 40);
        let telling = telling_frame(&starts_empty).expect("it moves");
        assert!(!blank(&telling) && telling.get_pixel(0, 0).0[0].abs_diff(90) <= 4, "{:?}", telling.get_pixel(0, 0));
        let (small, whole, _) = small_of(&starts_empty, 920).unwrap();
        assert!(!blank(&small), "the thumbnail shows the page, not the empty start");
        assert_eq!(whole, (64, 40));
        // and so is the one picture shown large where nothing may move
        assert!(!blank(&large_of(&starts_empty, (2000, 2000)).unwrap()));
        // one that shows something at once is known by its first picture
        let telling = telling_frame(&recording(4, 64, 40)).unwrap();
        assert!(telling.get_pixel(0, 0).0[0].abs_diff(40) <= 4);
        // one that never shows anything is still a picture: its first
        assert!(blank(&telling_frame(&recording_of(&[255, 255, 255], 64, 40)).unwrap()));
    }

    /// Phase 26: a recording plays in its thumbnail only while it is in sight.
    #[test]
    fn a_thumbnail_goes_on_only_after_a_paint_that_saw_it() {
        let sight = Sight::default();
        assert!(!may_go_on(false, &sight, 0), "never painted: it waits");
        sight.painted(false);
        assert!(!may_go_on(false, &sight, 0), "painted outside what the window shows: it waits");
        sight.painted(true);
        assert!(may_go_on(false, &sight, 0), "a paint saw it: it goes on");
        // the next picture went on screen after two paints: no paint has seen that one yet
        assert!(!may_go_on(false, &sight, sight.count()), "a window that isn't drawn never gets further");
        sight.painted(true);
        assert!(may_go_on(false, &sight, 2));
        assert!(!may_go_on(true, &sight, 2), "behind a screenshot shown large it waits");
        // scrolled away, then back
        sight.painted(false);
        assert!(!may_go_on(false, &sight, 2));
        sight.painted(true);
        assert!(may_go_on(false, &sight, 2));
    }

    /// Phase 26: the thumbnail's own pictures are cut as the thumbnail is, and a file says its kind by its bytes.
    #[test]
    fn a_thumbnail_plays_pictures_cut_as_it_is_cut_and_a_file_says_its_kind() {
        // 640 by 200 is wider than 16 to 10: the thumbnail keeps 320 by 200 from its left, and so does each picture
        let wide = recording(4, 640, 200);
        let (small, whole, covers) = small_of(&wide, 920).unwrap();
        assert_eq!((whole, covers), ((640, 200), false));
        let steps = play_small(Arc::new(wide.clone()), 920);
        let first: Vec<Step> = (0..4).map(|_| steps.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
        assert!(first.iter().all(|(pixels, _)| pixels.dimensions() == small.dimensions()), "{:?}", small.dimensions());
        for ((pixels, _), want) in first.iter().zip([40u8, 90, 140, 190]) {
            assert!(red(pixels).abs_diff(want) <= 4, "{} for {want}", red(pixels));
        }
        // never wider than asked
        let narrow = play_small(Arc::new(recording(3, 64, 40)), 32);
        assert_eq!(narrow.recv_timeout(Duration::from_secs(10)).unwrap().0.dimensions(), (32, 20));
        // the kind is read from the file, whatever its name says
        let mut png = Vec::new();
        let still = RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        image::DynamicImage::ImageRgba8(still).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!((kind_of(&png), kind_of(&wide), kind_of(b"not a picture")), ("PNG", "GIF", ""));
    }
}
