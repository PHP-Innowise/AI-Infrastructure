<?php
/** Trusted positive test fixture; never mount into candidate namespaces. */
namespace Psr\Cache {
    interface CacheItemInterface {
        public function getKey(): string;
        public function get(): mixed;
        public function isHit(): bool;
        public function set(mixed $value): static;
        public function expiresAfter(\DateInterval|int|null $time): static;
    }
    interface CacheItemPoolInterface {
        public function getItem(string $key): CacheItemInterface;
        public function save(CacheItemInterface $item): bool;
    }
}
namespace Symfony\Component\HttpFoundation {
    final class Request {
        public function __construct(private ?string $ip) {}
        public function getClientIp(): ?string { return $this->ip; }
    }
    final class RequestStack {
        public function __construct(public ?Request $request = null) {}
        public function getCurrentRequest(): ?Request { return $this->request; }
    }
}
namespace Regression {
    use Psr\Cache\CacheItemInterface;
    use Psr\Cache\CacheItemPoolInterface;
    use Symfony\Component\HttpFoundation\Request;
    use Symfony\Component\HttpFoundation\RequestStack;
    use App\Service\EntryFetchThrottle;
    final class Clock { public static int $now = 1000000; }
    final class Item implements CacheItemInterface {
        public ?int $ttl = null; // A fresh fetched object never inherits expiry.
        public function __construct(private string $key, private mixed $value, private bool $hit) {}
        public function getKey(): string { return $this->key; }
        public function get(): mixed { return $this->value; }
        public function isHit(): bool { return $this->hit; }
        public function set(mixed $value): static { $this->value = $value; return $this; }
        public function expiresAfter(\DateInterval|int|null $time): static {
            if (!is_int($time)) { throw new \RuntimeException('TTL must be an integer'); }
            $this->ttl = $time;
            return $this;
        }
    }
    final class Pool implements CacheItemPoolInterface {
        public array $values = [];
        public array $reads = [];
        public array $writes = [];
        public function getItem(string $key): CacheItemInterface {
            $this->reads[] = $key;
            return new Item($key, $this->values[$key] ?? null, array_key_exists($key, $this->values));
        }
        public function save(CacheItemInterface $item): bool {
            if (!$item instanceof Item) { throw new \RuntimeException('Unexpected cache item'); }
            $this->values[$item->getKey()] = $item->get();
            $this->writes[] = [$item->getKey(), $item->get(), $item->ttl];
            return true;
        }
        public function readsFor(string $key): int { return count(array_filter($this->reads, fn($k) => $k === $key)); }
        public function writesFor(string $key): array { return array_values(array_filter($this->writes, fn($w) => $w[0] === $key)); }
    }
    const GLOBAL_KEY = 'car_financing_entry_fetch_global';
    const T = 1000000;
    function key(string $ip): string { return 'car_financing_entry_fetch_' . sha1($ip); }
    function check(mixed $expected, mixed $actual, string $scenario): void {
        if ($expected !== $actual) {
            throw new \RuntimeException($scenario . ': expected ' . var_export($expected, true) . ', got ' . var_export($actual, true));
        }
    }
    function setup(?string $ip = '203.0.113.7'): array {
        Clock::$now = T;
        $pool = new Pool();
        $stack = new RequestStack(new Request($ip));
        return [$pool, $stack, new EntryFetchThrottle($pool, $stack)];
    }
    function lastWrite(Pool $pool, string $key): array {
        $writes = $pool->writesFor($key);
        return $writes[count($writes) - 1];
    }
    function row(Pool $pool, string $key, int $count, int $start, int $ttl, string $scenario): void {
        check([$key, [$count, $start], $ttl], lastWrite($pool, $key), $scenario);
    }
    function run(): void {
        [$pool, $stack, $throttle] = setup();
        $admitted = 0;
        for ($i = 1; $i <= 200; ++$i) {
            $allowed = $throttle->allow();
            check($i <= 60, $allowed, 'same request occurrence ' . $i);
            $admitted += (int) $allowed;
        }
        check(60, $admitted, '200 occurrences count');
        check([60, T], $pool->values[key('203.0.113.7')], 'caller count stops at cap');
        check(60, count($pool->writesFor(key('203.0.113.7'))), 'denials do not save caller');
        check(60, $pool->readsFor(GLOBAL_KEY), 'caller denials do not read global');
        check(60, count($pool->writesFor(GLOBAL_KEY)), 'caller denials do not save global');
        $stack->request = new Request('203.0.113.7');
        check(false, $throttle->allow(), 'new request same IP retains budget');
        $stack->request = new Request('198.51.100.4');
        check(true, $throttle->allow(), 'different IP retains independent budget');
        $second = new EntryFetchThrottle($pool, $stack);
        for ($i = 1; $i < 60; ++$i) { check(true, $second->allow(), 'shared instance admitted ' . $i); }
        check(false, $throttle->allow(), 'instances share caller count');

        [$pool, $stack, $throttle] = setup();
        for ($caller = 0; $caller < 100; ++$caller) {
            $stack->request = new Request('caller-' . $caller);
            for ($i = 0; $i < 60; ++$i) {
                check(true, $throttle->allow(), 'global exact 6000 caller ' . $caller . ' call ' . $i);
            }
        }
        $stack->request = new Request('fresh-after-global');
        check(false, $throttle->allow(), 'global refuses call 6001');
        check([1, T], $pool->values[key('fresh-after-global')], 'global denial spends caller first');
        check([6000, T], $pool->values[GLOBAL_KEY], 'global count stops at cap');
        check(6000, count($pool->writesFor(GLOBAL_KEY)), 'global denial does not save');

        [$pool, $stack, $throttle] = setup();
        $pool->values[key('203.0.113.7')] = [60, T];
        $pool->values[GLOBAL_KEY] = [5999, T];
        check(false, $throttle->allow(), 'exhausted caller with one global unit');
        check(0, $pool->readsFor(GLOBAL_KEY), 'exhausted caller skips global read');
        check([], $pool->writesFor(GLOBAL_KEY), 'exhausted caller skips global write');
        $stack->request = new Request('other');
        check(true, $throttle->allow(), 'remaining global unit available to other');
        check([6000, T], $pool->values[GLOBAL_KEY], 'last global unit spent exactly once');

        [$pool, $stack, $throttle] = setup();
        $pool->values[key('203.0.113.7')] = [3, T];
        $pool->values[GLOBAL_KEY] = [6000, T];
        check(false, $throttle->allow(), 'global exhausted with caller budget');
        row($pool, key('203.0.113.7'), 4, T, 60, 'caller charged before global denial');
        check([], $pool->writesFor(GLOBAL_KEY), 'global denial no rewrite');
        check([6000, T], $pool->values[GLOBAL_KEY], 'global denial unchanged counter');

        [$pool, $stack, $throttle] = setup();
        foreach ([0 => 60, 10 => 50, 59 => 1] as $offset => $ttl) {
            Clock::$now = T + $offset;
            check(true, $throttle->allow(), 'TTL allowed at offset ' . $offset);
            $count = $offset === 0 ? 1 : ($offset === 10 ? 2 : 3);
            row($pool, key('203.0.113.7'), $count, T, $ttl, 'caller fixed TTL ' . $offset);
            row($pool, GLOBAL_KEY, $count, T, $ttl, 'global fixed TTL ' . $offset);
        }
        // Retain expired saved pairs as cache hits; the service must reset them.
        $pool->values[key('203.0.113.7')] = [60, T];
        $pool->values[GLOBAL_KEY] = [6000, T];
        check(false, $throttle->allow(), 'caller remains closed at T+59');
        Clock::$now = T + 60;
        check(true, $throttle->allow(), 'both reset on T+60 retained cache hit');
        row($pool, key('203.0.113.7'), 1, T + 60, 60, 'caller boundary reset');
        row($pool, GLOBAL_KEY, 1, T + 60, 60, 'global boundary reset');

        [$pool, $stack, $throttle] = setup();
        $pool->values[GLOBAL_KEY] = [6000, T];
        Clock::$now = T + 59;
        check(false, $throttle->allow(), 'global stays closed T+59');
        check([], $pool->writesFor(GLOBAL_KEY), 'global closed no save T+59');
        Clock::$now = T + 60;
        check(true, $throttle->allow(), 'global resets at exact T+60');
        row($pool, GLOBAL_KEY, 1, T + 60, 60, 'global exact boundary retained hit');

        [$pool, $stack, $throttle] = setup();
        $pool->values[key('203.0.113.7')] = [4, T - 10];
        $pool->values[GLOBAL_KEY] = [9, T - 20];
        check(true, $throttle->allow(), 'valid pairs continue');
        row($pool, key('203.0.113.7'), 5, T - 10, 50, 'caller own earlier origin');
        row($pool, GLOBAL_KEY, 10, T - 20, 40, 'global own earlier origin');
        Clock::$now = T + 40;
        check(true, $throttle->allow(), 'global expires before caller');
        row($pool, key('203.0.113.7'), 6, T - 10, 10, 'caller unaffected by global reset');
        row($pool, GLOBAL_KEY, 1, T + 40, 60, 'global independent reset');
        Clock::$now = T + 50;
        check(true, $throttle->allow(), 'caller expires while global active');
        row($pool, key('203.0.113.7'), 1, T + 50, 60, 'caller independent reset');
        row($pool, GLOBAL_KEY, 2, T + 40, 50, 'global unaffected by caller reset');

        foreach ([null, false, 42, 'not-an-array'] as $nonarray) {
            [$pool, $stack, $throttle] = setup();
            $pool->values[key('203.0.113.7')] = $nonarray;
            $pool->values[GLOBAL_KEY] = $nonarray;
            check(true, $throttle->allow(), 'nonarray cache hit begins bucket');
            row($pool, key('203.0.113.7'), 1, T, 60, 'nonarray caller starts fresh');
            row($pool, GLOBAL_KEY, 1, T, 60, 'nonarray global starts fresh');
        }
        [$pool, $stack, $throttle] = setup();
        check(true, $throttle->allow(), 'cache miss begins bucket');
        row($pool, key('203.0.113.7'), 1, T, 60, 'cache miss caller');
        row($pool, GLOBAL_KEY, 1, T, 60, 'cache miss global');

        [$pool, $stack, $throttle] = setup();
        for ($i = 0; $i < 62; ++$i) {
            $stack->request = $i % 2 === 0 ? null : new Request(null);
            check($i < 60, $throttle->allow(), 'mixed fallback shared occurrence ' . $i);
        }
        check([60, T], $pool->values[GLOBAL_KEY], 'fallback spends global');
        $stack->request = new Request('ordinary-ip');
        check(true, $throttle->allow(), 'ordinary IP independent from fallback');
        check([61, T], $pool->values[GLOBAL_KEY], 'ordinary and fallback share global');
    }
}
namespace App\Service {
    function time(): int { return \Regression\Clock::$now; }
}
namespace {
    try {
        if ($argc !== 2 || !is_file($argv[1]) || !is_readable($argv[1])) {
            throw new \RuntimeException('Usage: php throttle_regression.php <readable EntryFetchThrottle.php path>');
        }
        require realpath($argv[1]);
        \Regression\run();
        echo "PASS all deterministic throttle scenarios\n";
    } catch (\Throwable $error) {
        fwrite(STDERR, 'FAIL scenario: ' . $error->getMessage() . "\n");
        exit(1);
    }
}
