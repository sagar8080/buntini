// Opt-in native smoke test. Creates one fresh, empty profile, never loads a URL,
// and removes that exact profile through WebKit before reporting success.
#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <dlfcn.h>
#import <objc/message.h>

static void runLoopUntil(BOOL (^done)(void), NSDate *deadline) {
    while (!done() && deadline.timeIntervalSinceNow > 0) {
        [[NSRunLoop currentRunLoop]
            runMode:NSDefaultRunLoopMode
            beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
    }
}

int main(int argc, const char *argv[]) {
    (void)argv;
    if (argc != 1) {
        fputs("This probe takes no arguments and only creates a fresh disposable profile.\n", stderr);
        return 2;
    }
    @autoreleasepool {
        if (@available(macOS 14.0, *)) {
            [NSApplication sharedApplication];
            NSUUID *identifier = [NSUUID UUID];
            BOOL verified = NO;
            @autoreleasepool {
                WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
                configuration.websiteDataStore = [WKWebsiteDataStore dataStoreForIdentifier:identifier];
                WKWebView *view = [[WKWebView alloc]
                    initWithFrame:NSMakeRect(0, 0, 200, 200)
                    configuration:configuration];
                WKWebsiteDataStore *store = view.configuration.websiteDataStore;
                WKPreferences *preferences = view.configuration.preferences;
                BOOL safe = store.persistent && [store.identifier isEqual:identifier];

                NSArray *settings = @[
                    @[@"_setPeerConnectionEnabled:", @"_peerConnectionEnabled"],
                    @[@"_setMediaDevicesEnabled:", @"_mediaDevicesEnabled"],
                    @[@"_setScreenCaptureEnabled:", @"_screenCaptureEnabled"]
                ];
                for (NSArray *setting in settings) {
                    SEL setter = NSSelectorFromString(setting[0]);
                    SEL getter = NSSelectorFromString(setting[1]);
                    if (![preferences respondsToSelector:setter] || ![preferences respondsToSelector:getter]) {
                        safe = NO;
                        break;
                    }
                    ((void (*)(id, SEL, BOOL))objc_msgSend)(preferences, setter, NO);
                    // Read a fresh configuration to check the live view shares
                    // the preferences object, rather than only a detached copy.
                    if (((BOOL (*)(id, SEL))objc_msgSend)(view.configuration.preferences, getter)) safe = NO;
                }

                void *library = dlopen("/System/Library/Frameworks/Network.framework/Network", RTLD_NOW | RTLD_LOCAL);
                id (*endpoint)(const char *, const char *) = library ? dlsym(library, "nw_endpoint_create_host") : NULL;
                id (*proxy)(id, id) = library ? dlsym(library, "nw_proxy_config_create_http_connect") : NULL;
                void (*setFailover)(id, bool) = library ? dlsym(library, "nw_proxy_config_set_failover_allowed") : NULL;
                bool (*getFailover)(id) = library ? dlsym(library, "nw_proxy_config_get_failover_allowed") : NULL;
                if (!endpoint || !proxy || !setFailover || !getFailover) safe = NO;
                if (safe) {
                    // The endpoint is never contacted: no URL is loaded and no
                    // listener is started. Only the native configuration is read.
                    id gate = proxy(endpoint("127.0.0.1", "9"), nil);
                    safe = gate != nil;
                    if (safe) {
                        setFailover(gate, false);
                        store.proxyConfigurations = @[gate];
                        NSArray *installed = view.configuration.websiteDataStore.proxyConfigurations;
                        safe = installed.count == 1 && installed[0] == gate && !getFailover(installed[0]);
                    }
                }
                // Keep Network loaded until process exit; native configuration
                // objects can survive this lexical scope during deferred close.
                verified = safe;
                printf("blank_native_isolation=%s\n", safe ? "PASS" : "FAIL");
            }

            // WebKit completes WKWebView teardown on a later main-run-loop turn.
            runLoopUntil(^BOOL { return NO; }, [NSDate dateWithTimeIntervalSinceNow:0.5]);
            NSDate *cleanupDeadline = [NSDate dateWithTimeIntervalSinceNow:10];
            BOOL removed = NO;
            for (unsigned attempt = 0; attempt < 3 && !removed; attempt++) {
                __block BOOL finished = NO;
                __block BOOL failed = NO;
                [WKWebsiteDataStore removeDataStoreForIdentifier:identifier completionHandler:^(NSError *error) {
                    finished = YES;
                    failed = error != nil;
                }];
                runLoopUntil(^BOOL { return finished; }, cleanupDeadline);
                if (!finished) break; // Unknown completion is not retried.
                removed = !failed;
                if (!removed && attempt < 2) {
                    NSDate *pause = [NSDate dateWithTimeIntervalSinceNow:0.25];
                    runLoopUntil(^BOOL { return cleanupDeadline.timeIntervalSinceNow <= 0; }, pause);
                }
                if (cleanupDeadline.timeIntervalSinceNow <= 0) break;
            }
            printf("disposable_profile_removed=%s\n", removed ? "PASS" : "FAIL");
            if (!removed) printf("disposable_profile_needing_cleanup=%s\n", identifier.UUIDString.UTF8String);
            return verified && removed ? 0 : 1;
        }
        puts("SKIP: browser isolation requires macOS 14 or later.");
        return 77;
    }
}
