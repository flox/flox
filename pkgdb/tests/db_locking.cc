
#include <fstream>

#include "flox/core/util.hh"
#include "flox/pkgdb/write.hh"
#include "nix/hash.hh"
#include "test.hh"

/* -------------------------------------------------------------------------- */

using namespace flox;
using namespace flox::pkgdb;

/* -------------------------------------------------------------------------- */

class TestDbLock : public DbLock
{
public:

  using DbLock::DbLock;
  using DbLock::getPID;
  using DbLock::readPIDsFromLock;
  using DbLock::registerInterest;
  using DbLock::setDbLockPath;
  using DbLock::shouldTakeOverDbCreation;
  using DbLock::unregisterInterest;
  using DbLock::waitForLockActivity;
  using DbLock::wasAbleToCreateDbLock;
  using DbLock::writePIDsToLock;
};

Fingerprint
dummyFingerprint()
{
  return nix::hashString( nix::htSHA256, "fingerprint" );
}

void
touchDbLock( const std::filesystem::path & path )
{
  std::ofstream lockfile;
  lockfile.open( path, std::ios_base::out );
  lockfile.close();
}

TestDbLock
dbLockAtRandomPath()
{
  Fingerprint fingerprint = dummyFingerprint();
  TestDbLock  lock( fingerprint );
  auto        lockPath
    = std::filesystem::temp_directory_path() / std::to_string( std::rand() );
  lock.setDbLockPath( lockPath );
  return lock;
}


/* -------------------------------------------------------------------------- */

bool
test_writesAndReadsPID()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  std::vector<pid_t> pidsToWrite = { 1, 2, 3, 4, 5 };
  lock.writePIDsToLock( pidsToWrite );
  auto pidsRead = lock.readPIDsFromLock();
  return *pidsRead == pidsToWrite;
}

bool
test_detectsShouldTakeOverDbCreation()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  /* With only process waiting on this lock, we should always be the process
   * that should take over creation of the database.*/
  lock.registerInterest();
  return lock.shouldTakeOverDbCreation();
}

bool
test_detectsShouldntTakeOverDbCreation()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  std::vector<pid_t> dummyPIDs = { 0 };
  lock.writePIDsToLock( dummyPIDs );
  /* Since we haven't registered interest in the lock we should never be the one
   * responsible for creating the database. */
  return ! lock.shouldTakeOverDbCreation();
}

bool
test_detectsStaleDbLock()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  /* sleep longer than the time it takes to become stale */
  std::this_thread::sleep_for( 1.5 * DB_LOCK_MAX_UPDATE_AGE );
  auto result = lock.waitForLockActivity();
  return result == DB_LOCK_ACTIVITY_WRITER_DIED;
}

bool
test_detectsDeletedDbLock()
{
  TestDbLock lock = dbLockAtRandomPath();
  std::filesystem::remove( lock.getDbLockPath() );
  auto result = lock.waitForLockActivity();
  return result == DB_LOCK_ACTIVITY_DELETED;
}

bool
test_waitsForLockActivity()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  auto now    = std::filesystem::last_write_time( lock.getDbLockPath() );
  auto result = lock.waitForLockActivity();
  auto later  = std::chrono::file_clock::now();
  EXPECT_EQ( result, DB_LOCK_ACTIVITY_WRITER_DIED );
  auto durationWaited = later - now;
  return durationWaited > DB_LOCK_TOUCH_INTERVAL;
}

bool
test_registersAndUnregistersLockInterest()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  lock.registerInterest();
  auto pids = lock.readPIDsFromLock();
  auto it   = std::find( pids->begin(), pids->end(), lock.getPID() );
  EXPECT( it != pids->end() );
  lock.unregisterInterest();
  pids = lock.readPIDsFromLock();
  it   = std::find( pids->begin(), pids->end(), lock.getPID() );
  return it == pids->end();
}

bool
test_detectsExistingLock()
{
  TestDbLock lock = dbLockAtRandomPath();
  touchDbLock( lock.getDbLockPath() );
  return ! lock.wasAbleToCreateDbLock();
}

/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int ec = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( ec, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )
    {
      nix::verbosity = nix::lvlDebug;
    }

  {
    RUN_TEST( writesAndReadsPID );
    RUN_TEST( detectsShouldTakeOverDbCreation );
    RUN_TEST( detectsShouldntTakeOverDbCreation );
    RUN_TEST( detectsStaleDbLock );
    RUN_TEST( detectsDeletedDbLock );
    RUN_TEST( waitsForLockActivity );
    RUN_TEST( registersAndUnregistersLockInterest );
    RUN_TEST( detectsExistingLock );
  }

  return ec;
}
