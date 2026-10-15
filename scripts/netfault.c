// netfault: make a running canon lose, and fail to regain, a connection to a device, on demand.
//
// Some failures only happen on real hardware and can't be provoked from canon's side: a speaker
// closing its Cast session, or refusing connections while its Cast service restarts (canon-5303).
// This shim, preloaded into canon (Linux, LD_PRELOAD; no root needed), fakes both against a real
// device, keyed by the device's port and switched by files under target/netfault/ (relative to
// canon's working directory):
//
//   drop-<port>    the next read from a connection to <port> finds it closed (shut down, so the
//                  reader sees EOF); the file is removed, so it fires once
//   refuse-<port>  every connect to <port> is refused (ECONNREFUSED) while the file exists
//   ignore-<port>  every connect to <port> hangs for 130s, then times out (ETIMEDOUT), as one to a
//                  device that ignores TCP does (canon-9bce)
//
//   cc -shared -fPIC -O2 -o target/netfault.so scripts/netfault.c -ldl
//   mkdir -p target/netfault
//   LD_PRELOAD=$PWD/target/netfault.so ./target/debug/canon serve --bind 127.0.0.1:7399 ...
//   touch target/netfault/refuse-8009 target/netfault/drop-8009

#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dlfcn.h>
#include <errno.h>
#include <netinet/in.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

static int port_of(const struct sockaddr *addr) {
    if (addr->sa_family == AF_INET) {
        return ntohs(((const struct sockaddr_in *)addr)->sin_port);
    }
    if (addr->sa_family == AF_INET6) {
        return ntohs(((const struct sockaddr_in6 *)addr)->sin6_port);
    }
    return -1;
}

static int armed(const char *what, int port) {
    char path[64];
    snprintf(path, sizeof path, "target/netfault/%s-%d", what, port);
    return access(path, F_OK) == 0;
}

int connect(int fd, const struct sockaddr *addr, socklen_t len) {
    static int (*real)(int, const struct sockaddr *, socklen_t);
    if (!real) {
        real = dlsym(RTLD_NEXT, "connect");
    }
    int port = port_of(addr);
    if (port > 0 && armed("refuse", port)) {
        fprintf(stderr, "netfault: refusing a connect to port %d\n", port);
        errno = ECONNREFUSED;
        return -1;
    }
    if (port > 0 && armed("ignore", port)) {
        fprintf(stderr, "netfault: ignoring a connect to port %d\n", port);
        sleep(130);
        errno = ETIMEDOUT;
        return -1;
    }
    return real(fd, addr, len);
}

ssize_t recv(int fd, void *buf, size_t len, int flags) {
    static ssize_t (*real)(int, void *, size_t, int);
    if (!real) {
        real = dlsym(RTLD_NEXT, "recv");
    }
    struct sockaddr_storage peer;
    socklen_t peer_len = sizeof peer;
    if (getpeername(fd, (struct sockaddr *)&peer, &peer_len) == 0) {
        int port = port_of((struct sockaddr *)&peer);
        char path[64];
        snprintf(path, sizeof path, "target/netfault/drop-%d", port);
        if (port > 0 && unlink(path) == 0) {
            fprintf(stderr, "netfault: dropping the connection to port %d\n", port);
            shutdown(fd, SHUT_RDWR);
        }
    }
    return real(fd, buf, len, flags);
}
