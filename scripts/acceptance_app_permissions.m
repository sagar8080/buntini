// No-prompt acceptance preflight: asks only permission state, never app content.
#import <Cocoa/Cocoa.h>
#import <ApplicationServices/ApplicationServices.h>
#import <EventKit/EventKit.h>

int main(void) {
    @autoreleasepool {
        NSMutableDictionary *apps = [NSMutableDictionary dictionary];
        for (NSString *bundle in @[@"com.apple.Notes", @"com.apple.reminders", @"com.apple.iCal", @"com.apple.Safari", @"com.google.Chrome", @"com.brave.Browser"]) {
            AEAddressDesc target = {typeNull, NULL};
            NSData *data = [bundle dataUsingEncoding:NSUTF8StringEncoding];
            OSStatus status = AECreateDesc(typeApplicationBundleID, data.bytes, data.length, &target);
            if (status == noErr) {
                status = AEDeterminePermissionToAutomateTarget(&target, kAECoreSuite, kAEGetData, false);
                AEDisposeDesc(&target);
            }
            NSString *state = status == noErr ? @"authorized" : status == errAEEventWouldRequireUserConsent ? @"consent_required" : status == errAEEventNotPermitted ? @"denied" : @"unavailable_or_unknown";
            apps[bundle] = @{@"automation":state, @"status":@(status), @"installed":@([[NSWorkspace sharedWorkspace] URLForApplicationWithBundleIdentifier:bundle] != nil)};
        }
        NSDictionary *report = @{
            @"kind":@"no_prompt_app_permission_preflight",
            @"apps":apps,
            @"eventKitCalendarStatus":@([EKEventStore authorizationStatusForEntityType:EKEntityTypeEvent]),
            @"eventKitRemindersStatus":@([EKEventStore authorizationStatusForEntityType:EKEntityTypeReminder]),
            @"accessibilityAuthorized":@(AXIsProcessTrusted()),
            @"personalContentRead":@NO,
            @"permissionPromptsRequested":@NO
        };
        NSData *json = [NSJSONSerialization dataWithJSONObject:report options:NSJSONWritingPrettyPrinted error:nil];
        puts([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
    }
    return 0;
}
