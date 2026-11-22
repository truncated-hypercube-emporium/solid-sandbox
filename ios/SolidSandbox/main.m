// Solid Sandbox for iOS: a full-screen view that hands the Rust core
// (src/ios.rs) its taps, time and accelerometer, and shows the pixels it
// draws. No storyboard; landscape only.

#import <UIKit/UIKit.h>
#import <CoreMotion/CoreMotion.h>
#import <QuartzCore/QuartzCore.h>
#include "solid_sandbox.h"

@interface SandboxViewController : UIViewController
@end

@implementation SandboxViewController {
    SSApp *_app;
    UIView *_canvas;          // the sandbox, inside the safe area
    CADisplayLink *_link;
    CFTimeInterval _last;     // when the last frame was stepped
    uint32_t *_pixels;
    int32_t _w, _h;           // pixel size of _pixels
    CGColorSpaceRef _space;
    CMMotionManager *_motion;
}

- (void)loadView {
    UIView *root = [[UIView alloc] init];
    root.backgroundColor = UIColor.blackColor;
    _canvas = [[UIView alloc] init];
    _canvas.backgroundColor = UIColor.blackColor;
    _canvas.layer.magnificationFilter = kCAFilterNearest;
    _canvas.layer.contentsGravity = kCAGravityResize;
    [root addSubview:_canvas];
    self.view = root;
    _space = CGColorSpaceCreateDeviceRGB();
    _motion = [[CMMotionManager alloc] init];
    _motion.accelerometerUpdateInterval = 1.0 / 50.0;
}

- (void)viewDidLoad {
    [super viewDidLoad];
    _link = [CADisplayLink displayLinkWithTarget:self selector:@selector(tick:)];
    [_link addToRunLoop:NSRunLoop.mainRunLoop forMode:NSRunLoopCommonModes];
    NSNotificationCenter *nc = NSNotificationCenter.defaultCenter;
    [nc addObserver:self selector:@selector(pause) name:UIApplicationWillResignActiveNotification object:nil];
    [nc addObserver:self selector:@selector(resume) name:UIApplicationDidBecomeActiveNotification object:nil];
}

- (void)pause {
    _link.paused = YES;
    [_motion stopAccelerometerUpdates];
}

- (void)resume {
    _last = 0;
    _link.paused = NO;
}

- (NSString *)dataDirectory {
    NSFileManager *fm = NSFileManager.defaultManager;
    NSURL *dir = [[fm URLsForDirectory:NSApplicationSupportDirectory inDomains:NSUserDomainMask] firstObject];
    [fm createDirectoryAtURL:dir withIntermediateDirectories:YES attributes:nil error:nil];
    return dir.path;
}

// Fit the canvas to the safe area (clear of the notch and rounded corners)
// and the pixel buffer and the app to the canvas.
- (void)viewDidLayoutSubviews {
    [super viewDidLayoutSubviews];
    _canvas.frame = UIEdgeInsetsInsetRect(self.view.bounds, self.view.safeAreaInsets);
    CGFloat scale = self.view.window.screen.scale ?: UIScreen.mainScreen.scale;
    int32_t w = (int32_t)lround(_canvas.bounds.size.width * scale);
    int32_t h = (int32_t)lround(_canvas.bounds.size.height * scale);
    if (w <= 0 || h <= 0 || (w == _w && h == _h)) {
        return;
    }
    free(_pixels);
    _pixels = calloc((size_t)w * (size_t)h, sizeof(uint32_t));
    _w = w;
    _h = h;
    if (_app) {
        ss_resize(_app, w, h);
    } else {
        _app = ss_create(w, h, self.dataDirectory.fileSystemRepresentation);
    }
}

// Android's Surface.ROTATION_* for the current interface orientation.
- (int32_t)rotation {
    switch (self.view.window.windowScene.interfaceOrientation) {
        case UIInterfaceOrientationLandscapeRight: return 1;
        case UIInterfaceOrientationPortraitUpsideDown: return 2;
        case UIInterfaceOrientationLandscapeLeft: return 3;
        default: return 0;
    }
}

- (void)tick:(CADisplayLink *)link {
    if (!_app || !_pixels) {
        return;
    }
    // the app's frame-rate cap
    float fps = ss_fps(_app);
    CFTimeInterval now = link.timestamp;
    if (_last == 0) {
        _last = now - 1.0 / fps;
    }
    CFTimeInterval dt = now - _last;
    if (dt < 1.0 / fps - 0.002) {
        return;
    }
    _last = now;

    // the accelerometer runs only while gravity is on
    bool wants = ss_wants_accel(_app);
    if (wants && !_motion.accelerometerActive && _motion.accelerometerAvailable) {
        [_motion startAccelerometerUpdates];
    } else if (!wants && _motion.accelerometerActive) {
        [_motion stopAccelerometerUpdates];
    }
    CMAccelerometerData *a = wants ? _motion.accelerometerData : nil;
    ss_step(_app, (float)dt, a != nil, (float)a.acceleration.x, (float)a.acceleration.y, (float)a.acceleration.z, self.rotation);

    ss_draw(_app, _pixels, _w, _w, _h);
    CGContextRef ctx = CGBitmapContextCreate(_pixels, (size_t)_w, (size_t)_h, 8, (size_t)_w * 4, _space,
                                             kCGImageAlphaNoneSkipLast | kCGBitmapByteOrder32Big);
    if (ctx) {
        CGImageRef image = CGBitmapContextCreateImage(ctx);
        _canvas.layer.contents = (__bridge id)image;
        CGImageRelease(image);
        CGContextRelease(ctx);
    }
}

- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    if (!_app) {
        return;
    }
    CGFloat scale = _w / MAX(_canvas.bounds.size.width, 1.0);
    for (UITouch *t in touches) {
        CGPoint p = [t locationInView:_canvas];
        ss_tap(_app, (float)(p.x * scale), (float)(p.y * scale));
    }
}

- (BOOL)prefersStatusBarHidden {
    return YES;
}

- (BOOL)prefersHomeIndicatorAutoHidden {
    return YES;
}

- (UIInterfaceOrientationMask)supportedInterfaceOrientations {
    return UIInterfaceOrientationMaskLandscape;
}

- (void)dealloc {
    [_link invalidate];
    [_motion stopAccelerometerUpdates];
    ss_destroy(_app);
    free(_pixels);
    CGColorSpaceRelease(_space);
}

@end

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)options {
    self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];
    self.window.rootViewController = [[SandboxViewController alloc] init];
    [self.window makeKeyAndVisible];
    application.idleTimerDisabled = YES; // keep the screen on, like the Android build
    return YES;
}

@end

int main(int argc, char *argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass(AppDelegate.class));
    }
}