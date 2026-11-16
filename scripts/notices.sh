#!/bin/sh
# Writes, to standard output, the notices of the packages the app is built from on this Mac: each package cargo
# builds into the app and its command, with the license it names, and then every license text those packages carry,
# each text once with the packages that carry it. A release carries this file (scripts/package.sh), since a program
# that is handed out must bring its licenses with it. The packages are read where cargo keeps them
# (~/.cargo/registry/src), so build once before this.
#
#   sh scripts/notices.sh > THIRD-PARTY-NOTICES.txt

set -eu
cd "$(dirname "$0")/.."
cargo tree --locked -p enkidu-app -p enkidu-cli -e normal --prefix none --format '{p}|{l}|{r}' \
  | sed -e 's/ (\*)$//' | sort -u | perl -e '
  use strict; use warnings;
  my $registry = "$ENV{HOME}/.cargo/registry/src";
  my (@packages, %text, %carriers, %order, %seen);
  while (my $line = <STDIN>) {
    chomp $line;
    next unless $line =~ /\S/;
    my ($package, $license, $repository) = split /\|/, $line, 3;
    # a package of this repository says "name v1.0 (/path)": its license is the app s own
    next if $package =~ /\(\//;
    my ($name, $version) = $package =~ /^(\S+) v(\S+)/ or next;
    next if $seen{"$name $version"}++;
    $license ||= "no license named"; $repository ||= "";
    my @files;
    for my $dir (glob("$registry/*/$name-$version")) {
      for my $file (sort glob("$dir/*")) {
        next unless -f $file;
        my ($base) = $file =~ m{([^/]+)$};
        push @files, $file if $base =~ /^(?:LICEN[SC]E|COPYING|COPYRIGHT|UNLICENSE|NOTICE)/i;
      }
      last if @files;
    }
    push @packages, [$name, $version, $license, $repository, scalar @files];
    for my $file (@files) {
      open my $in, "<:raw", $file or next;
      my $body = do { local $/; <$in> }; close $in;
      $body =~ s/\r\n?/\n/g; $body =~ s/[ \t]+\n/\n/g; $body =~ s/\A\s+|\s+\z//g;
      next unless length $body;
      # one text, however a package wraps or indents it
      (my $key = lc $body) =~ s/\s+/ /g;
      $text{$key} //= $body;
      $order{$key} //= scalar keys %order;
      push @{ $carriers{$key} }, "$name $version";
    }
  }
  print "Enkidu: the packages it is built from\n\n";
  print "Enkidu itself is under the license in the file LICENSE, and NOTICE names what was copied into it. The app is\n";
  print "built from the packages below. Each keeps its own license. Their license texts follow the list, each text once,\n";
  print "with the packages that carry it. A package that carries no text of its own is under the license named beside it.\n\n";
  print scalar(@packages), " packages\n\n";
  for my $p (sort { lc $a->[0] cmp lc $b->[0] || $a->[1] cmp $b->[1] } @packages) {
    printf "%s %s\n    license: %s%s\n", $p->[0], $p->[1], $p->[2], $p->[4] ? "" : " (no text of its own in the package)";
    print "    $p->[3]\n" if length $p->[3];
  }
  print "\n\nThe license texts\n";
  for my $key (sort { $order{$a} <=> $order{$b} } keys %text) {
    my %seen; my @who = grep { !$seen{$_}++ } @{ $carriers{$key} };
    print "\n", "=" x 100, "\n";
    print "Carried by ", scalar(@who), (@who == 1 ? " package" : " packages"), ": ", join(", ", @who), "\n";
    print "=" x 100, "\n\n";
    print $text{$key}, "\n";
  }
'

# the language server the app carries as a program of its own, when it was fetched (scripts/fetch-ty.sh; NOTICE
# names it): its own license text
ty_license=$(ls target/tools/ty/lib/python*/site-packages/ty-*.dist-info/licenses/LICENSE 2>/dev/null | head -1 || true)
if [ -n "$ty_license" ] && [ -x target/tools/ty/bin/ty ]; then
  printf '\n%s\n' "===================================================================================================="
  echo "Carried inside the app as a program of its own: ty $(target/tools/ty/bin/ty --version | awk '{print $2}') (Astral)"
  printf '%s\n\n' "===================================================================================================="
  cat "$ty_license"
fi
