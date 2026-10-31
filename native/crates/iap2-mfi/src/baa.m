// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

#import <Foundation/Foundation.h>
#import <Security/Security.h>
#import <Security/SecTask.h>
#include <dlfcn.h>

// Private ABI observed in CarPlay Simulator 4.0's CarPlaySDK. Resolve it at
// runtime so machines without this service still run LIVI's other backends.
typedef void (*IssueCertificate)(CFTypeRef, CFDictionaryRef,
                                void (^)(SecKeyRef, CFArrayRef, CFErrorRef));

@interface LIVIBAAIdentity : NSObject
@property(nonatomic) SecKeyRef key;
@property(nonatomic, strong) NSData *leaf;
@property(nonatomic, strong) NSData *intermediate;
@end

@implementation LIVIBAAIdentity
- (void)dealloc {
    if (_key) CFRelease(_key);
}
@end

static void *deviceIdentity;
static IssueCertificate issueCertificate;

static NSString *option(const char *name) {
    CFStringRef *symbol = dlsym(deviceIdentity, name);
    return symbol ? (__bridge NSString *)*symbol : nil;
}

static void failure(char *error, size_t capacity, NSString *message) {
    if (capacity) snprintf(error, capacity, "%s", message.UTF8String);
}

static BOOL available(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        deviceIdentity = dlopen("/System/Library/PrivateFrameworks/DeviceIdentity.framework/DeviceIdentity",
                                RTLD_LAZY | RTLD_LOCAL);
        if (deviceIdentity) {
            issueCertificate = (IssueCertificate)dlsym(deviceIdentity,
                "DeviceIdentityIssueClientCertificateWithCompletion");
        }
    });
    if (!issueCertificate) return NO;
    const char *names[] = {
        "kMAOptionsBAAKeychainLabel", "kMAOptionsBAAIgnoreExistingKeychainItems",
        "kMAOptionsBAANetworkTimeoutInterval", "kMAOptionsBAAValidity",
        "kMAOptionsBAASCRTAttestation", "kMAOptionsBAAAccessControls",
        "kMAOptionsBAAOIDSToInclude", "kMAOptionsBAAOIDUCRTDeviceIdentifiers"
    };
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (!option(names[i])) return NO;
    }
    return YES;
}

bool livi_baa_available(void) {
    @autoreleasepool { return available(); }
}

bool livi_baa_has_entitlements(void) {
    @autoreleasepool {
        SecTaskRef task = SecTaskCreateFromSelf(NULL);
        if (!task) return false;
        bool allowed = true;
        const CFStringRef names[] = {
            CFSTR("com.apple.mobileactivationd.deviceidentity-allowlist-exempt"),
            CFSTR("com.apple.mobileactivationd.spi"),
            CFSTR("com.apple.mobileactivationd.device-identifiers"),
            CFSTR("com.apple.security.attestation.access"),
        };
        for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
            CFTypeRef value = SecTaskCopyValueForEntitlement(task, names[i], NULL);
            allowed = allowed && value && CFEqual(value, kCFBooleanTrue);
            if (value) CFRelease(value);
        }
        CFTypeRef appID = SecTaskCopyValueForEntitlement(task, CFSTR("application-identifier"), NULL);
        allowed = allowed && appID && CFGetTypeID(appID) == CFStringGetTypeID()
                  && CFStringGetLength((CFStringRef)appID) > 0;
        if (appID) CFRelease(appID);
        CFRelease(task);
        return allowed;
    }
}

void *livi_baa_create(char *error, size_t capacity) {
    @autoreleasepool {
        if (!available()) {
            failure(error, capacity, @"DeviceIdentity BAA service is unavailable");
            return NULL;
        }
        CFErrorRef accessError = NULL;
        SecAccessControlRef access = SecAccessControlCreateWithFlags(NULL,
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAccessControlPrivateKeyUsage, &accessError);
        if (!access) {
            failure(error, capacity, accessError ? [(__bridge NSError *)accessError description]
                                                : @"Cannot create key access control");
            if (accessError) CFRelease(accessError);
            return NULL;
        }
        NSDictionary *options = @{
            option("kMAOptionsBAAKeychainLabel"): @"dev.f-io.livi.baa",
            option("kMAOptionsBAAIgnoreExistingKeychainItems"): @NO,
            option("kMAOptionsBAANetworkTimeoutInterval"): @60,
            option("kMAOptionsBAAValidity"): @43200,
            option("kMAOptionsBAASCRTAttestation"): @NO,
            option("kMAOptionsBAAAccessControls"): (__bridge id)access,
            option("kMAOptionsBAAOIDSToInclude"):
                @[option("kMAOptionsBAAOIDUCRTDeviceIdentifiers")],
        };
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block LIVIBAAIdentity *identity = nil;
        __block NSString *reason = nil;
        // The callback may outlive the bounded wait. ARC keeps its captured state
        // alive without retaining any caller-owned output buffers.
        issueCertificate(NULL, (__bridge CFDictionaryRef)options,
            ^(SecKeyRef key, CFArrayRef chain, CFErrorRef err) {
                @autoreleasepool {
                    if (err) {
                        reason = [(__bridge NSError *)err description];
                    } else if (!key || !chain || CFArrayGetCount(chain) != 2) {
                        reason = @"DeviceIdentity returned an incomplete certificate chain";
                    } else {
                        identity = [LIVIBAAIdentity new];
                        identity.key = (SecKeyRef)CFRetain(key);
                        identity.leaf = CFBridgingRelease(SecCertificateCopyData(
                            (SecCertificateRef)CFArrayGetValueAtIndex(chain, 0)));
                        identity.intermediate = CFBridgingRelease(SecCertificateCopyData(
                            (SecCertificateRef)CFArrayGetValueAtIndex(chain, 1)));
                    }
                    dispatch_semaphore_signal(done);
                }
            });
        CFRelease(access);
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 65 * NSEC_PER_SEC))) {
            failure(error, capacity, @"Timed out obtaining the device certificate");
            return NULL;
        }
        if (!identity.leaf.length || !identity.intermediate.length) {
            failure(error, capacity, reason ?: @"DeviceIdentity returned no certificates");
            return NULL;
        }
        return (__bridge_retained void *)identity;
    }
}

void livi_baa_free(void *handle) {
    @autoreleasepool { CFBridgingRelease(handle); }
}

// All returned byte buffers are malloc-owned and freed by livi_baa_bytes_free.
static bool copyData(NSData *data, uint8_t **bytes, size_t *length) {
    if (!data.length) return false;
    *bytes = malloc(data.length);
    if (!*bytes) return false;
    memcpy(*bytes, data.bytes, data.length);
    *length = data.length;
    return true;
}

bool livi_baa_certificate(void *handle, bool intermediate, uint8_t **bytes, size_t *length) {
    @autoreleasepool {
        LIVIBAAIdentity *identity = (__bridge LIVIBAAIdentity *)handle;
        return copyData(intermediate ? identity.intermediate : identity.leaf, bytes, length);
    }
}

bool livi_baa_sign(void *handle, const uint8_t *challenge, size_t length,
                   uint8_t **bytes, size_t *outLength, char *error, size_t capacity) {
    @autoreleasepool {
        LIVIBAAIdentity *identity = (__bridge LIVIBAAIdentity *)handle;
        NSData *message = [NSData dataWithBytes:challenge length:length];
        CFErrorRef signError = NULL;
        // BAA hashes the message here and returns an X9.62 DER signature. Hardware
        // MFi instead accepts a digest and returns a fixed-width signature.
        NSData *signature = CFBridgingRelease(SecKeyCreateSignature(identity.key,
            kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
            (__bridge CFDataRef)message, &signError));
        if (!signature) {
            failure(error, capacity, signError ? [(__bridge NSError *)signError description]
                                              : @"Signing failed");
            if (signError) CFRelease(signError);
            return false;
        }
        return copyData(signature, bytes, outLength);
    }
}

void livi_baa_bytes_free(uint8_t *bytes) { free(bytes); }
